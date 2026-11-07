//! The stand-in against Apple's `codesign`, through the files
//! `fixtures/codesign/regen.py` makes with it: the same inputs, signed with
//! the same test identity, must come out the same, but for the CMS
//! signature, which carries the time it was made.

use super::*;
use crate::cli::testdir::TempDir;
use rsa::traits::PublicKeyParts as _;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/codesign")
}

fn fixture_identity() -> Identity {
    let data = std::fs::read(fixtures().join("identity.p12")).unwrap();
    Identity::from_p12(&data, "sweetpad-test", true).unwrap()
}

fn copy_tree(from: &Path, to: &Path) {
    if from.is_dir() {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            copy_tree(&entry.path(), &to.join(entry.file_name()));
        }
    } else {
        std::fs::copy(from, to).unwrap();
    }
}

/// Sign `name` from the inputs into `dir`, as Swift Build asks `codesign`
/// to, and return where it is.
fn sign(dir: &Path, name: &str, identity: Option<&Identity>, entitlements: bool) -> PathBuf {
    let path = dir.join(name);
    copy_tree(
        &fixtures().join("input").join(name.replace(".adhoc", "")),
        &path,
    );
    let options = Options {
        identity: identity.map_or("-".to_string(), |i| i.certificate.sha1_hex()),
        entitlements: entitlements.then(|| fixtures().join("input/app.xcent")),
        der_entitlements: true,
        ..Options::default()
    };
    let signer = Signer {
        options: &options,
        identity,
        time: SystemTime::now(),
    };
    if path.is_dir() {
        signer.sign_bundle(&path).unwrap();
    } else {
        signer.sign_file(&path, None).unwrap();
    }
    path
}

/// A slice's code before its signature, the signature's blobs by slot, and
/// the slice's size.
type Parts = (Vec<u8>, Vec<(u32, Vec<u8>)>, usize);

/// The parts of each slice.
fn signature_parts(data: &[u8]) -> Vec<Parts> {
    macho::split(data)
        .unwrap()
        .into_iter()
        .map(|slice| {
            let layout = macho::Layout::parse(&slice.data).unwrap();
            let superblob = layout.signature(&slice.data).unwrap();
            let be = |at: usize| u32::from_be_bytes(superblob[at..at + 4].try_into().unwrap());
            let blobs = (0..be(8) as usize)
                .map(|i| {
                    let (slot, at) = (be(12 + 8 * i), be(16 + 8 * i) as usize);
                    (slot, superblob[at..at + be(at + 4) as usize].to_vec())
                })
                .collect();
            (
                slice.data[..layout.signature_offset].to_vec(),
                blobs,
                slice.data.len(),
            )
        })
        .collect()
}

/// That `ours` is signed as `expected` is, but for the CMS signature,
/// which must still sign `ours`'s code directory with the identity's key.
fn assert_same_signature(ours: &Path, expected: &Path, identity: Option<&Identity>) {
    let (ours_data, expected_data) = (
        std::fs::read(ours).unwrap(),
        std::fs::read(expected).unwrap(),
    );
    let (ours_parts, expected_parts) =
        (signature_parts(&ours_data), signature_parts(&expected_data));
    assert_eq!(ours_parts.len(), expected_parts.len());
    for ((code, blobs, length), (expected_code, expected_blobs, expected_length)) in
        ours_parts.iter().zip(&expected_parts)
    {
        let name = ours.display();
        assert_eq!(length, expected_length, "{name}: the signed size differs");
        assert!(code == expected_code, "{name}: the code or headers differ");
        let slots = |blobs: &[(u32, Vec<u8>)]| blobs.iter().map(|(s, _)| *s).collect::<Vec<_>>();
        assert_eq!(
            slots(blobs),
            slots(expected_blobs),
            "{name}: the blobs differ"
        );
        for ((slot, blob), (_, expected_blob)) in blobs.iter().zip(expected_blobs) {
            if *slot != blobs::SLOT_CMS {
                assert!(blob == expected_blob, "{name}: blob {slot:#x} differs");
            }
        }
        let directory = &blobs[0].1;
        let cms = &blobs.last().unwrap().1[8..];
        match identity {
            Some(identity) => verify_cms(cms, directory, identity),
            None => assert!(cms.is_empty()),
        }
    }
}

/// That `cms` signs `directory`'s hash with `identity`'s key.
fn verify_cms(cms: &[u8], directory: &[u8], identity: &Identity) {
    use der::Reader;
    let info = Reader::new(cms).read().unwrap();
    let mut fields = info.children();
    fields.read().unwrap();
    let signed = fields.read().unwrap().children().read().unwrap();
    let signer_info = signed
        .children()
        .last()
        .unwrap()
        .unwrap()
        .children()
        .read()
        .unwrap();
    let mut parts = signer_info.children();
    for _ in 0..3 {
        parts.read().unwrap();
    }
    let attributes = parts.read().unwrap();
    parts.read().unwrap();
    let signature = parts.read().unwrap().octets().unwrap();
    let digest = crypto::sha256(directory);
    assert!(
        attributes.raw.windows(32).any(|w| w == digest),
        "the CMS doesn't carry the code directory's hash"
    );
    let mut set = attributes.as_der();
    set[0] = der::SET;
    let key = rsa::RsaPublicKey::from(identity.key.as_ref().unwrap());
    let prefix = [
        0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01,
        0x05, 0x00, 0x04, 0x20,
    ];
    let hashed = [&prefix[..], &crypto::sha256(&set)].concat();
    key.verify(rsa::Pkcs1v15Sign::new_unprefixed(), &hashed, &signature)
        .expect("the CMS signature doesn't verify");
    assert_eq!(key.n().to_bytes_be(), identity.certificate.modulus);
}

#[test]
fn signs_an_executable_and_a_library_like_codesign() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-files");
    for name in ["Tool", "Library.dylib", "LldTool"] {
        let ours = sign(&dir, name, Some(&identity), false);
        assert_same_signature(
            &ours,
            &fixtures().join("expected").join(name),
            Some(&identity),
        );
    }
}

#[test]
fn signs_ad_hoc_like_codesign() {
    let dir = TempDir::new("sweetpad-codesign-adhoc");
    let ours = sign(&dir, "Library.adhoc.dylib", None, false);
    assert_same_signature(
        &ours,
        &fixtures().join("expected/Library.adhoc.dylib"),
        None,
    );
}

#[test]
fn signs_a_bundle_like_codesign() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-bundle");
    let ours = sign(&dir, "App.app", Some(&identity), true);
    let expected = fixtures().join("expected/App.app");
    for file in [
        "_CodeSignature/CodeResources",
        "PlugIns/Extension.appex/_CodeSignature/CodeResources",
        "Info.plist",
    ] {
        assert!(
            std::fs::read(ours.join(file)).unwrap() == std::fs::read(expected.join(file)).unwrap(),
            "{file} differs"
        );
    }
    assert_same_signature(&ours.join("App"), &expected.join("App"), Some(&identity));
}

#[test]
fn resource_bundles_match_native_detached_signatures() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-resources");
    for (name, identity) in [
        ("Resources.bundle", Some(&identity)),
        ("Resources.adhoc.bundle", None),
    ] {
        let ours = sign(&dir, name, identity, true);
        let expected = fixtures().join("expected").join(name);
        for file in ["CodeDirectory", "CodeRequirements", "CodeResources"] {
            let relative = Path::new("_CodeSignature").join(file);
            assert_eq!(
                std::fs::read(ours.join(&relative)).unwrap(),
                std::fs::read(expected.join(&relative)).unwrap(),
                "{name}: {file} differs from Apple's signature"
            );
        }
        let directory = std::fs::read(ours.join("_CodeSignature/CodeDirectory")).unwrap();
        let cms = std::fs::read(ours.join("_CodeSignature/CodeSignature")).unwrap();
        if let Some(identity) = identity {
            verify_cms(&cms, &directory, identity);
        } else {
            assert!(cms.is_empty());
        }
        #[cfg(target_os = "macos")]
        {
            let verify = || {
                std::process::Command::new("/usr/bin/codesign")
                    .args(["--verify", "--strict"])
                    .arg(&ours)
                    .output()
                    .unwrap()
            };
            assert!(verify().status.success());
            std::fs::write(ours.join("message.txt"), "tampered").unwrap();
            assert!(
                !verify().status.success(),
                "a changed resource must fail verification"
            );
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn apples_codesign_verifies_what_it_signs() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-verify");
    let app = sign(&dir, "App.app", Some(&identity), true);
    let tool = sign(&dir, "Tool", Some(&identity), false);
    for path in [app, tool] {
        let status = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success(), "codesign rejects {}", path.display());
    }
}

#[test]
fn reads_keychain_and_openssl_exports_alike() {
    let identity = fixture_identity();
    let aes = std::fs::read(fixtures().join("identity-aes.p12")).unwrap();
    let other = Identity::from_p12(&aes, "sweetpad-test", true).unwrap();
    assert_eq!(identity.certificate.der, other.certificate.der);
    assert!(identity.matches("Apple Development"));
    assert!(identity.matches("iPhone Developer"));
    assert!(identity.matches(&identity.certificate.sha1_hex()));
    assert!(!identity.matches("Apple Distribution"));
    assert!(identity.is_development());
    let p12 = std::fs::read(fixtures().join("identity.p12")).unwrap();
    let error = Identity::from_p12(&p12, "nope", true).err().unwrap();
    assert!(error.contains("wrong password"), "{error}");
}

#[test]
fn an_apple_issued_identity_gets_xcodes_designated_requirement() {
    let identity = apple_issued_fixture();
    assert!(identity.is_apple_issued());
    assert_eq!(identity.team(), Some("TESTTEAM01"));
    // identifier "id" and anchor apple generic and certificate
    // leaf[subject.CN] = "…" and certificate 1[field.1.2.840.113635.100.6.2.1]
    // exists, as Xcode's signature of an app encodes it.
    let name = b"Apple Development: Sweetpad Fixture (TESTTEAM01)";
    let mut expression: Vec<u8> = Vec::new();
    for word in [6u32, 2, 2] {
        expression.extend(word.to_be_bytes());
    }
    expression.extend(b"id\0\0");
    for word in [6u32, 15, 6, 11, 0, 10] {
        expression.extend(word.to_be_bytes());
    }
    expression.extend(b"subject.CN\0\0");
    expression.extend(1u32.to_be_bytes());
    expression.extend(u32::try_from(name.len()).unwrap().to_be_bytes());
    expression.extend(name);
    expression.resize(expression.len().next_multiple_of(4), 0);
    for word in [14u32, 1, 10] {
        expression.extend(word.to_be_bytes());
    }
    expression.extend([
        0x2a, 0x86, 0x48, 0x86, 0xf7, 0x63, 0x64, 0x06, 0x02, 0x01, 0, 0,
    ]);
    expression.extend(0u32.to_be_bytes());
    let requirement = [&1u32.to_be_bytes()[..], &expression].concat();
    let requirement = blobs::blob(blobs::REQUIREMENT, &requirement);
    let set = [
        &1u32.to_be_bytes()[..],
        &3u32.to_be_bytes(),
        &20u32.to_be_bytes(),
        &requirement,
    ]
    .concat();
    assert_eq!(
        blobs::requirements("id", Some(&identity)).unwrap(),
        blobs::blob(blobs::REQUIREMENTS, &set)
    );
}

fn request(entries: &[(&str, Plist)]) -> Plist {
    let mut request = vec![
        (
            "bundleIdentifier",
            Plist::String("dev.sweetpad.fixture".to_string()),
        ),
        (
            "signingCertificateIdentifier",
            Plist::String("Apple Development".to_string()),
        ),
        (
            "entitlementsDestination",
            Plist::String("Signature".to_string()),
        ),
        (
            "provisioningProfileSupport",
            Plist::String("required".to_string()),
        ),
        ("provisioningStyle", Plist::String("automatic".to_string())),
        ("sdkRoot", Plist::String("iphoneos27.0".to_string())),
        ("teamID", Plist::String("TESTTEAM01".to_string())),
        ("productTypeEntitlements", Plist::Dict(Vec::new())),
    ];
    for (key, value) in entries {
        request.retain(|(k, _)| k != key);
        request.push((key, value.clone()));
    }
    Plist::Dict(
        request
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

/// The fixture identity, as if Apple had issued it: what provisioning
/// reads a team from.
fn apple_issued_fixture() -> Identity {
    let mut identity = fixture_identity();
    identity.chain = [
        include_bytes!("certs/AppleWWDRCAG3.cer").as_slice(),
        include_bytes!("certs/AppleRootCA.cer"),
    ]
    .iter()
    .map(|der| identity::Certificate::parse(der).unwrap())
    .collect();
    identity
}

fn provision(request: &Plist) -> Result<Plist, String> {
    provisioning::answer(request, &[fixtures().join("profiles")], || {
        Ok(Some(apple_issued_fixture()))
    })
}

#[test]
fn provisions_like_xcodes_automatic_signing() {
    let keychain = Plist::Dict(vec![(
        "keychain-access-groups".to_string(),
        Plist::Array(vec![Plist::String(
            "$(AppIdentifierPrefix)dev.sweetpad.fixture".to_string(),
        )]),
    )]);
    let inputs = provision(&request(&[("projectEntitlements", keychain)])).unwrap();
    let identity = fixture_identity();
    assert_eq!(
        plist::string(&inputs, "identityHash"),
        Some(identity.certificate.sha1_hex().as_str())
    );
    assert_eq!(
        plist::string(&inputs, "profileName"),
        Some("Sweetpad Fixture Wildcard")
    );
    assert!(
        plist::string(&inputs, "profilePath")
            .unwrap()
            .ends_with(".mobileprovision")
    );
    assert_eq!(
        plist::string(&inputs, "appIdentifierPrefix"),
        Some("TESTTEAM01.")
    );
    let signed = plist::get(&inputs, "signedEntitlements").unwrap();
    assert_eq!(
        plist::string(signed, "application-identifier"),
        Some("TESTTEAM01.dev.sweetpad.fixture")
    );
    assert_eq!(
        plist::string(signed, "com.apple.developer.team-identifier"),
        Some("TESTTEAM01")
    );
    assert_eq!(
        plist::get(signed, "get-task-allow"),
        Some(&Plist::Bool(true))
    );
    // Swift Build evaluates the prefix itself, from the inputs.
    assert_eq!(
        plist::get(signed, "keychain-access-groups"),
        Some(&Plist::Array(vec![Plist::String(
            "$(AppIdentifierPrefix)dev.sweetpad.fixture".to_string()
        )]))
    );
    assert_eq!(
        plist::get(&inputs, "simulatedEntitlements"),
        Some(&Plist::Dict(Vec::new()))
    );
}

#[test]
fn provisioning_problems_are_xcodes_errors() {
    let manual = provision(&request(&[
        ("provisioningStyle", Plist::String("manual".to_string())),
        (
            "provisioningProfileSpecifier",
            Plist::String("Sweetpad Fixture Wildcard".to_string()),
        ),
    ]))
    .unwrap();
    assert_eq!(
        plist::string(&manual, "profileName"),
        Some("Sweetpad Fixture Wildcard")
    );

    let team = provision(&request(&[(
        "teamID",
        Plist::String("OTHERTEAM1".to_string()),
    )]));
    assert!(team.unwrap_err().contains("belongs to team TESTTEAM01"));
    let no_team = provision(&request(&[("teamID", Plist::String(String::new()))])).unwrap();
    assert_eq!(
        plist::string(&no_team, "teamIdentifierPrefix"),
        Some("TESTTEAM01.")
    );
    let missing = provision(&request(&[
        (
            "bundleIdentifier",
            Plist::String("dev.sweetpad.fixture".to_string()),
        ),
        ("sdkRoot", Plist::String("appletvos27.0".to_string())),
    ]));
    assert!(
        missing
            .unwrap_err()
            .starts_with("No profiles for 'dev.sweetpad.fixture' were found")
    );
    let domains = Plist::Dict(vec![(
        "com.apple.developer.associated-domains".to_string(),
        Plist::Array(vec![Plist::String("applinks:example.com".to_string())]),
    )]);
    let entitlement = provision(&request(&[("projectEntitlements", domains)]));
    assert_eq!(
        entitlement.unwrap_err(),
        "Provisioning profile \"Sweetpad Fixture Wildcard\" doesn't include the \
         com.apple.developer.associated-domains entitlement."
    );
    let distribution = provision(&request(&[(
        "signingCertificateIdentifier",
        Plist::String("Apple Distribution".to_string()),
    )]));
    assert!(
        distribution
            .unwrap_err()
            .contains("CODE_SIGN_IDENTITY is 'Apple Distribution'")
    );
}

#[test]
fn ad_hoc_and_unsigned_need_no_identity() {
    let no_identity = || Err("not needed".to_string());
    let adhoc = request(&[(
        "signingCertificateIdentifier",
        Plist::String("-".to_string()),
    )]);
    let inputs = provisioning::answer(&adhoc, &[], no_identity).unwrap();
    assert_eq!(plist::string(&inputs, "identityHash"), Some("-"));
    let signed = plist::get(&inputs, "signedEntitlements").unwrap();
    assert_eq!(
        plist::string(signed, "application-identifier"),
        Some("dev.sweetpad.fixture")
    );
    let unsigned = request(&[("signingCertificateIdentifier", Plist::String(String::new()))]);
    assert_eq!(
        provisioning::answer(&unsigned, &[], no_identity).unwrap(),
        Plist::Dict(Vec::new())
    );
}

#[test]
fn derq_writes_the_der_entitlements_codesign_signs() {
    let dir = TempDir::new("sweetpad-derq");
    let output = dir.join("app.xcent.der");
    let args: Vec<String> = [
        "query",
        "-f",
        "xml",
        "-i",
        &fixtures().join("input/app.xcent").display().to_string(),
        "-o",
        &output.display().to_string(),
        "--raw",
    ]
    .iter()
    .map(ToString::to_string)
    .collect();
    assert_eq!(derq_main(&args), ExitCode::SUCCESS);
    let expected = std::fs::read(fixtures().join("expected/App.app/App")).unwrap();
    let parts = signature_parts(&expected);
    let (_, blob) = parts[0]
        .1
        .iter()
        .find(|(slot, _)| *slot == blobs::SLOT_DER_ENTITLEMENTS)
        .unwrap();
    assert_eq!(std::fs::read(&output).unwrap(), blob[8..]);
}
