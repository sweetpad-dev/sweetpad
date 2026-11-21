//! Signing in to Apple ID from a terminal: SRP, the two-factor code, and
//! the hand-off that makes the session good for Apple Developer downloads.

use base64::Engine as _;
use serde_json::{Value, json};

use super::http::{Call, Reply, header, put};
use super::session::Session;
use super::srp::{Srp, hashcash};
use super::{AUTH, url};

/// Asks the user something; the flag hides the answer.
pub(crate) type Prompt<'a> = &'a mut dyn FnMut(&str, bool) -> Result<String, String>;

pub(crate) struct Login<'a> {
    pub(crate) session: &'a mut Session,
    pub(crate) prompt: Prompt<'a>,
    pub(crate) note: &'a mut dyn FnMut(&str),
    /// Headers Apple's sign-in hands back and expects on the next request.
    pub(crate) headers: Vec<(String, String)>,
    /// The pause before a GET is retried, doubled the second time.
    pub(crate) retry_pause: std::time::Duration,
}

impl<'a> Login<'a> {
    pub(crate) fn new(
        session: &'a mut Session,
        prompt: Prompt<'a>,
        note: &'a mut dyn FnMut(&str),
    ) -> Self {
        Self {
            session,
            prompt,
            note,
            headers: Vec::new(),
            retry_pause: std::time::Duration::from_secs(2),
        }
    }

    fn auth(
        &mut self,
        method: &'static str,
        path: &str,
        body: Option<Value>,
    ) -> Result<Reply, String> {
        // Apple's sign-in gateway answers a GET with a 502–504 now and then;
        // a GET here changes nothing and has no body, so it is sent again
        // after a pause.
        let mut body = body;
        let mut attempt = 0;
        let reply = loop {
            let mut call = Call::new(method, format!("{AUTH}{path}")).headers(&self.headers);
            if let Some(body) = body.take() {
                call = call.json(body);
            }
            let reply = self.session.client.request(call)?;
            attempt += 1;
            if method != "GET" || !matches!(reply.0, 502..=504) || attempt == 3 {
                break reply;
            }
            std::thread::sleep(self.retry_pause * attempt);
        };
        for key in ["scnt", "X-Apple-ID-Session-Id"] {
            if let Some(value) = header(&reply.1, key).filter(|v| !v.is_empty()) {
                put(&mut self.headers, key, value);
            }
        }
        Ok(reply)
    }

    /// The public key Apple's sign-in widget uses. Asked for without the
    /// session's cookies and without following the redirect, which would
    /// sign the user out.
    pub(crate) fn service_key(&mut self) -> Result<String, String> {
        let client = &mut self.session.client;
        let (code, headers, _) = client.request(
            Call::new("HEAD", "https://appstoreconnect.apple.com/logout")
                .no_follow()
                .without_cookies(),
        )?;
        if matches!(code, 301 | 302 | 303 | 307 | 308)
            && let Some(location) = header(&headers, "location").filter(|l| url::apple(l))
            && let Some(key) = url::parse(location)
                .and_then(|u| u.query)
                .and_then(|q| url::query_value(q, "widgetKey"))
        {
            return Ok(key);
        }
        let (code, _, data) = client.request(
            Call::get(
                "https://appstoreconnect.apple.com/olympus/v1/app/config?hostname=itunesconnect.apple.com",
            )
            .without_cookies(),
        )?;
        match data.as_ref().and_then(|d| d["authServiceKey"].as_str()) {
            Some(key) if code == 200 && !key.is_empty() => Ok(key.to_string()),
            _ => Err("Couldn't get Apple's public sign-in key. Try again later.".into()),
        }
    }

    /// Apple's lookup of `account`, which starts a sign-in.
    fn lookup(&mut self, account: &str) -> Result<Value, String> {
        let (code, _, data) = self.auth(
            "POST",
            "/federate",
            Some(json!({"accountName": account, "rememberMe": false})),
        )?;
        data.filter(|d| code == 200 && d.is_object())
            .ok_or_else(|| format!("Apple's account lookup failed (HTTP {code})."))
    }

    /// The account lookup, the challenge and the SRP setup, which run back to
    /// back once the password is known: the hashcash proof, the SRP state and
    /// Apple's challenge. The inner error is Apple refusing the SRP setup.
    #[allow(clippy::type_complexity)] // the three parts the next step takes
    fn start(
        &mut self,
        account: &str,
        key: &str,
    ) -> Result<Result<(String, Srp, Value), String>, String> {
        self.lookup(account)?;
        let (code, headers, _) = self.auth(
            "GET",
            &format!("/signin?{}", url::form(&[("widgetKey", key)])),
            None,
        )?;
        if code != 200 {
            return Err(format!("Apple's sign-in challenge failed (HTTP {code})."));
        }
        let proof = hashcash(
            header(&headers, "X-Apple-HC-Bits"),
            header(&headers, "X-Apple-HC-Challenge"),
            None,
        )?;
        let srp = Srp::random()?;
        let b64 = base64::engine::general_purpose::STANDARD;
        let (code, _, challenge) = self.auth(
            "POST",
            "/signin/init",
            Some(json!({
                "accountName": account,
                "a": b64.encode(&srp.public),
                "protocols": ["s2k", "s2k_fo"],
            })),
        )?;
        Ok(match challenge {
            Some(challenge) if code == 200 && challenge["c"].is_string() => {
                Ok((proof, srp, challenge))
            }
            other => Err(format!(
                "Apple's SRP setup failed (HTTP {code}{}).",
                service_error(other.as_ref())
            )),
        })
    }

    #[allow(clippy::too_many_lines)] // one sign-in, in the order Apple's steps run
    pub(crate) fn login(&mut self, account: &str) -> Result<(), String> {
        let key = self.service_key()?;
        self.headers = vec![
            ("X-Apple-Widget-Key".into(), key.clone()),
            ("X-Requested-With".into(), "XMLHttpRequest".into()),
        ];
        if truthy(&self.lookup(account)?["federated"]) {
            return Err("Terminal sign-in doesn't support federated Apple Accounts.".into());
        }
        let password = (self.prompt)("Apple Account password: ", true)?;
        if password.is_empty() {
            return Err("The password is empty.".into());
        }
        // Apple refuses some SRP setups with -900007 and accepts the same
        // requests moments later. Nothing secret has been sent yet, so the
        // sign-in starts over once, from a clean session.
        let (proof, srp, challenge) = if let Ok(started) = self.start(account, &key)? {
            started
        } else {
            std::thread::sleep(self.retry_pause);
            self.session.client.cookies.clear();
            self.headers.retain(|(name, _)| {
                !name.eq_ignore_ascii_case("scnt")
                    && !name.eq_ignore_ascii_case("X-Apple-ID-Session-Id")
            });
            self.start(account, &key)??
        };
        let (m1, m2) = srp.proofs(account, &password, &challenge)?;
        drop(password);
        put(&mut self.headers, "X-Apple-HC", &proof);
        let (code, _, outcome) = self.auth(
            "POST",
            "/signin/complete?isRememberMeEnabled=false",
            Some(json!({
                "accountName": account,
                "c": challenge["c"],
                "m1": m1,
                "m2": m2,
                "rememberMe": false,
            })),
        )?;
        self.headers
            .retain(|(name, _)| !name.eq_ignore_ascii_case("X-Apple-HC"));
        let refusal = |what: &str, advice: &str| {
            format!(
                "{what} (HTTP {code}{}{}).{advice}",
                service_error(outcome.as_ref()),
                service_message(outcome.as_ref())
            )
        };
        match code {
            200 => {}
            409 => self.second_factor()?,
            401 => {
                return Err(refusal(
                    "Apple rejected the account or password",
                    " Check them before trying again.",
                ));
            }
            403 | 423 => {
                return Err(refusal(
                    "Apple blocked the sign-in",
                    " Review the account at account.apple.com before trying again.",
                ));
            }
            412 => {
                return Err(refusal(
                    "Apple needs its privacy and Developer agreements accepted",
                    " Accept them in a browser, then sign in again.",
                ));
            }
            _ => return Err(refusal("Apple's sign-in failed", "")),
        }
        self.session.apple_id = Some(account.to_string());
        // Apple's web clients ask for the Olympus session after SRP; that
        // request completes the hand-off Developer services rely on.
        let (code, _, data) = self.session.client.request(Call::get(
            "https://appstoreconnect.apple.com/olympus/v1/session",
        ))?;
        if code != 200 || !data.is_some_and(|d| d["user"].is_object()) {
            return Err(format!(
                "Apple couldn't set up the web session after sign-in (HTTP {code})."
            ));
        }
        // Keep the signed-in session even if Developer access needs an
        // agreement or the catalog is briefly down.
        self.session.save()?;
        let page = self.session.client.open(
            Call::get("https://developer.apple.com/download/").header("Accept", "text/html"),
        )?;
        if page.status != 200 {
            return Err(format!(
                "Apple Developer didn't accept the session (HTTP {}).",
                page.status
            ));
        }
        drop(page);
        self.session.catalog()?;
        self.session.save()
    }

    #[allow(clippy::too_many_lines)] // one verification, in the order Apple's steps run
    pub(crate) fn second_factor(&mut self) -> Result<(), String> {
        if header(&self.headers, "scnt").is_none()
            || header(&self.headers, "X-Apple-ID-Session-Id").is_none()
        {
            return Err("Apple left out the two-factor session headers.".into());
        }
        let (code, headers, options) = self.auth("GET", "", None)?;
        let Some(options) = options
            .as_ref()
            .filter(|o| (200..300).contains(&code) && o.is_object())
            .cloned()
        else {
            let kind = match options {
                Some(_) => service_error(options.as_ref()),
                None => format!(
                    ", {}",
                    header(&headers, "content-type").map_or("no body type", |t| {
                        t.split(';').next().unwrap_or(t).trim()
                    })
                ),
            };
            return Err(format!(
                "Couldn't read Apple's two-factor options (HTTP {code}{kind})."
            ));
        };
        if truthy(&options["fsaChallenge"]) {
            return Err("Terminal sign-in doesn't support security keys.".into());
        }
        if !options["trustedDevices"].is_null() {
            return Err(
                "Apple's older two-step verification isn't supported. Turn on \
                        two-factor authentication for the account."
                    .into(),
            );
        }
        let info = &options["securityCode"];
        if [
            "securityCodeLocked",
            "securityCodeCooldown",
            "tooManyCodesSent",
            "tooManyCodesValidated",
        ]
        .iter()
        .any(|key| truthy(&info[*key]))
        {
            return Err("Apple has locked verification codes for now. Try again later.".into());
        }
        let phones = options["trustedPhoneNumbers"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut phone = None;
        if truthy(&options["noTrustedDevices"]) {
            match phones.len() {
                0 => return Err("Apple returned no trusted device or phone number.".into()),
                // Apple sends the text to a single number on its own.
                1 => phone = Some(phone_id(&phones[0])?),
                _ => {
                    for (index, item) in phones.iter().enumerate() {
                        let number = item["numberWithDialCode"]
                            .as_str()
                            .unwrap_or("trusted phone");
                        (self.note)(&format!("{}: {number}", index + 1));
                    }
                    let choice = (self.prompt)("Send the code to which number? ", false)?;
                    let chosen = choice
                        .trim()
                        .parse::<usize>()
                        .ok()
                        .and_then(|n| n.checked_sub(1))
                        .and_then(|n| phones.get(n))
                        .ok_or("That isn't one of the listed numbers.")?;
                    let id = phone_id(chosen)?;
                    let (code, _, _) = self.auth(
                        "PUT",
                        "/verify/phone",
                        Some(json!({"phoneNumber": {"id": id}, "mode": "sms"})),
                    )?;
                    if !matches!(code, 200 | 204) {
                        return Err("Apple couldn't send the text message.".into());
                    }
                    phone = Some(id);
                }
            }
        }
        let value: String = (self.prompt)("Apple verification code: ", false)?
            .chars()
            .filter(|c| !matches!(c, ' ' | '-'))
            .collect();
        let length = info["length"].as_u64().unwrap_or(6);
        if value.is_empty()
            || !value.bytes().all(|b| b.is_ascii_digit())
            || value.len() as u64 != length
        {
            return Err(format!("The verification code must be {length} digits."));
        }
        let mut body = json!({"securityCode": {"code": value}});
        let channel = match phone {
            Some(id) => {
                body["phoneNumber"] = json!({"id": id});
                body["mode"] = json!("sms");
                "phone"
            }
            None => "trusteddevice",
        };
        let (code, _, _) = self.auth(
            "POST",
            &format!("/verify/{channel}/securitycode"),
            Some(body),
        )?;
        if !matches!(code, 200 | 204) {
            return Err(
                "Apple rejected the verification code. Sign in again with a new code.".into(),
            );
        }
        let (code, _, _) = self.auth("GET", "/2sv/trust", None)?;
        if !matches!(code, 200 | 204) {
            return Err("Apple couldn't trust this session.".into());
        }
        Ok(())
    }
}

/// `, code` for the first of Apple's `serviceErrors`, which name the
/// failure without the account's details.
fn service_error(data: Option<&Value>) -> String {
    data.and_then(|d| d["serviceErrors"][0]["code"].as_str())
        .filter(|code| code.bytes().all(|b| b.is_ascii_digit() || b == b'-') && code.len() <= 12)
        .map(|code| format!(", {code}"))
        .unwrap_or_default()
}

/// `: message` for the first of Apple's `serviceErrors`, Apple's own words
/// for the refusal, kept to one short printable line.
fn service_message(data: Option<&Value>) -> String {
    data.and_then(|d| d["serviceErrors"][0]["message"].as_str())
        .map(|message| {
            let line = message.split_whitespace().collect::<Vec<_>>().join(" ");
            let line: String = line.chars().filter(|c| !c.is_control()).take(200).collect();
            line.trim_end_matches('.').to_string()
        })
        .filter(|message| !message.is_empty())
        .map(|message| format!(": {message}"))
        .unwrap_or_default()
}

fn phone_id(phone: &Value) -> Result<Value, String> {
    match &phone["id"] {
        Value::Null => Err("Apple returned a trusted phone number without an id.".into()),
        id => Ok(id.clone()),
    }
}

/// Whether a JSON value counts as set, the way Apple's flags are read.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}
