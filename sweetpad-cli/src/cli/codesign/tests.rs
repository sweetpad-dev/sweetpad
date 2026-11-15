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

/// Apple's verifier needs the self-signed fixture certificate in an explicit
/// keychain. Import only its public certificate, and keep login keys untouched.
#[cfg(target_os = "macos")]
struct NativeVerifier {
    _lock: std::sync::MutexGuard<'static, ()>,
    dir: TempDir,
    search: Vec<String>,
}

#[cfg(target_os = "macos")]
impl NativeVerifier {
    fn new() -> Self {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let lock = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let listing = std::process::Command::new("security")
            .args(["list-keychains", "-d", "user"])
            .output()
            .unwrap();
        assert!(listing.status.success());
        let search = String::from_utf8(listing.stdout)
            .unwrap()
            .lines()
            .map(|s| s.trim().trim_matches('"').to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let verifier = Self {
            _lock: lock,
            dir: TempDir::new("sweetpad-public-verification"),
            search,
        };
        let certificate = verifier.dir.join("fixture.cer");
        std::fs::write(&certificate, fixture_identity().certificate.der).unwrap();
        let keychain = verifier.dir.join("fixture.keychain-db");
        for args in [
            vec!["create-keychain", "-p", "fixture"],
            vec!["unlock-keychain", "-p", "fixture"],
        ] {
            assert!(
                std::process::Command::new("security")
                    .args(args)
                    .arg(&keychain)
                    .output()
                    .unwrap()
                    .status
                    .success()
            );
        }
        let imported = std::process::Command::new("security")
            .arg("import")
            .arg(certificate)
            .arg("-k")
            .arg(&keychain)
            .output()
            .unwrap();
        assert!(imported.status.success());
        verifier
    }

    fn verify(&self, path: &Path, deep: bool) -> std::process::Output {
        let mut command = std::process::Command::new("/usr/bin/codesign");
        command
            .args(["--verify", "--strict", "--keychain"])
            .arg(self.dir.join("fixture.keychain-db"));
        if deep {
            command.arg("--deep");
        }
        command.arg(path).output().unwrap()
    }
}

#[cfg(target_os = "macos")]
impl Drop for NativeVerifier {
    fn drop(&mut self) {
        let _ = std::process::Command::new("security")
            .arg("delete-keychain")
            .arg(self.dir.join("fixture.keychain-db"))
            .output();
        let _ = std::process::Command::new("security")
            .args(["list-keychains", "-d", "user", "-s"])
            .args(&self.search)
            .output();
    }
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

/// The page size, the SHA-1 code directory beside the SHA-256 one, 32-bit
/// watch code, and the identifier an embedded Info.plist names, each as
/// Apple's `codesign` picks it.
#[test]
fn signs_each_platforms_code_like_codesign() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-platforms");
    for (name, identity) in [
        ("Watch.adhoc", None),
        ("Phone15.adhoc", None),
        ("Phone10.adhoc", None),
        ("Phone10", Some(&identity)),
        ("TV.adhoc", None),
        ("Info.adhoc.dylib", None),
    ] {
        let ours = sign(&dir, name, identity, false);
        assert_same_signature(&ours, &fixtures().join("expected").join(name), identity);
    }
}

#[test]
fn signing_a_bundle_executable_recognizes_its_identifier_and_resources() {
    let dir = TempDir::new("sweetpad-codesign-executable-path");
    let app = dir.join("App.app");
    copy_tree(&fixtures().join("input/App.app"), &app);
    let path = app.join("App");
    assert_eq!(bundle::of_executable(&path), Some(app.clone()));
    assert!(bundle::of_executable(&app.join("App.debug.dylib")).is_none());
    run(&[
        "--force".into(),
        "--sign".into(),
        "-".into(),
        "--".into(),
        path.to_string_lossy().into_owned(),
    ])
    .unwrap();
    let signature = signature_parts(&std::fs::read(&path).unwrap());
    let superblob = blobs::superblob(signature[0].1.clone());
    assert_eq!(
        blobs::read_existing(&superblob).unwrap().identifier,
        "dev.sweetpad.fixture"
    );
    assert!(app.join("_CodeSignature/CodeResources").is_file());
    // An executable path must use exactly the same bundle context as signing
    // the bundle directory, including every signed byte and resource hash.
    let control = sign(&dir.join("bundle-control"), "App.app", None, false);
    assert_same_signature(&path, &control.join("App"), None);
    #[cfg(target_os = "macos")]
    {
        let native = dir.join("Native.app");
        copy_tree(&fixtures().join("input/App.app"), &native);
        let result = std::process::Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-"])
            .arg(native.join("App"))
            .output()
            .unwrap();
        assert!(result.status.success());
        // Native signature allocation differs across macOS versions. Keep the
        // byte comparison above and the fixed native golden tests separate
        // from this host's verifier and resource-sealing compatibility check.
        let verified = std::process::Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(&app)
            .output()
            .unwrap();
        assert!(
            verified.status.success(),
            "{}",
            String::from_utf8_lossy(&verified.stderr)
        );
        let native_signature = signature_parts(&std::fs::read(native.join("App")).unwrap());
        let native_blobs = blobs::superblob(native_signature[0].1.clone());
        let native_metadata = blobs::read_existing(&native_blobs).unwrap();
        let metadata = blobs::read_existing(&superblob).unwrap();
        assert_eq!(metadata.identifier, native_metadata.identifier);
        assert_eq!(metadata.flags, native_metadata.flags);
        assert_eq!(
            std::fs::read(app.join("_CodeSignature/CodeResources")).unwrap(),
            std::fs::read(native.join("_CodeSignature/CodeResources")).unwrap()
        );
    }
}

#[test]
fn requirements_probe_reads_signed_and_adhoc_libraries_without_a_key() {
    for (name, expected) in [
        (
            "Library.dylib",
            "designated => identifier Library and certificate root = H\"8c4aecd312ff94abf07792060ef65967ad32c203\"\n",
        ),
        (
            "Library.adhoc.dylib",
            "# designated => cdhash H\"cdea7e0d39a2d6918711a99726d1d84504ae6bac\"\n",
        ),
    ] {
        let data = std::fs::read(fixtures().join("expected").join(name)).unwrap();
        assert_eq!(inspect::requirements(&data).unwrap(), expected);
        assert_eq!(
            std::fs::read(fixtures().join("expected").join(name)).unwrap(),
            data
        );
    }
    let unsigned = std::fs::read(fixtures().join("input/Library.dylib")).unwrap();
    assert!(inspect::requirements(&unsigned).is_err());
    let options = parse(&["-r-".into(), "--display".into(), "Library.dylib".into()]).unwrap();
    assert!(options.display && options.identity.is_empty());
    assert!(parse(&["--sign".into(), "-".into(), "-r-".into()]).is_err());
    let dir = TempDir::new("sweetpad-requirements-apple");
    let signed = sign(&dir, "Tool", Some(&apple_issued_fixture()), false);
    let data = std::fs::read(signed).unwrap();
    assert_eq!(
        inspect::requirements(&data).unwrap(),
        "designated => identifier Tool and anchor apple generic and certificate leaf[subject.CN] = \"Apple Development: Sweetpad Fixture (TESTTEAM01)\" and certificate 1[field.1.2.840.113635.100.6.2.1] /* exists */\n"
    );
    for length in [0, 4, 31, data.len() / 2] {
        assert!(inspect::requirements(&data[..length]).is_err());
    }
}

#[cfg(target_os = "macos")]
#[test]
fn requirements_probe_matches_apple_for_existing_signatures() {
    for name in ["Library.dylib", "Library.adhoc.dylib", "App.app/App"] {
        let path = fixtures().join("expected").join(name);
        let native = std::process::Command::new("/usr/bin/codesign")
            .args(["-r-", "--display"])
            .arg(&path)
            .output()
            .unwrap();
        assert!(native.status.success());
        assert_eq!(
            inspect::requirements(&std::fs::read(path).unwrap())
                .unwrap()
                .as_bytes(),
            native.stdout
        );
    }
    let dir = TempDir::new("sweetpad-requirements-native");
    let path = sign(&dir, "Tool", Some(&apple_issued_fixture()), false);
    let native = std::process::Command::new("/usr/bin/codesign")
        .args(["-r-", "--display"])
        .arg(&path)
        .output()
        .unwrap();
    assert!(native.status.success());
    assert_eq!(
        inspect::requirements(&std::fs::read(path).unwrap())
            .unwrap()
            .as_bytes(),
        native.stdout
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
fn a_bundle_around_sha1_code_seals_its_files_with_sha1_too() {
    let dir = TempDir::new("sweetpad-codesign-legacy");
    let ours = sign(&dir, "Legacy.adhoc.app", None, false);
    let expected = fixtures().join("expected/Legacy.adhoc.app");
    assert!(
        std::fs::read(ours.join("_CodeSignature/CodeResources")).unwrap()
            == std::fs::read(expected.join("_CodeSignature/CodeResources")).unwrap(),
        "CodeResources differs"
    );
    assert_same_signature(&ours.join("Legacy"), &expected.join("Legacy"), None);
}

#[test]
fn resource_bundles_match_native_detached_signatures() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-resources");
    #[cfg(target_os = "macos")]
    let verifier = NativeVerifier::new();
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
            let verify = || verifier.verify(&ours, false);
            let result = verify();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
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
    let verifier = NativeVerifier::new();
    for path in [app, tool] {
        let result = verifier.verify(&path, true);
        assert!(
            result.status.success(),
            "codesign rejects {}: {}",
            path.display(),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

/// An identity whose issuer neither its `.p12` file nor sweetpad carries
/// signs nothing, rather than signing with a chain that stops short.
#[test]
fn an_identity_whose_chain_stops_short_of_a_root_is_refused() {
    let data = std::fs::read(fixtures().join("orphan.p12")).unwrap();
    let orphan = Identity::from_p12(&data, "sweetpad-test", true).unwrap();
    let error = orphan.require_chain().unwrap_err();
    assert!(error.contains("Sweetpad Fixture Authority (G9)"), "{error}");
    assert!(fixture_identity().require_chain().is_ok());

    let dir = TempDir::new("sweetpad-codesign-orphan");
    let path = dir.join("Tool");
    std::fs::copy(fixtures().join("input/Tool"), &path).unwrap();
    let options = Options {
        identity: orphan.certificate.sha1_hex(),
        ..Options::default()
    };
    let signer = Signer {
        options: &options,
        identity: Some(&orphan),
        time: SystemTime::now(),
    };
    let error = signer.sign_file(&path, None).unwrap_err();
    assert!(error.contains("can't reach Apple's root"), "{error}");
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

/// The kind fixtures with the chain of an Apple-issued certificate, so
/// each names its team.
fn kind_set() -> identity::IdentitySet {
    let files = [
        fixtures().join("identities.p12"),
        fixtures().join("identity.p12"),
    ];
    let mut set = identity::IdentitySet::from_files(&files, "sweetpad-test", true).unwrap();
    for identity in &mut set.identities {
        identity.chain = apple_issued_fixture().chain;
    }
    set
}

#[test]
fn several_p12_files_make_one_set_picked_by_kind_and_team() {
    use identity::Kind;
    let set = kind_set();
    let names: Vec<&str> = set
        .identities
        .iter()
        .map(|i| i.certificate.common_name.as_str())
        .collect();
    assert_eq!(set.identities.len(), 5, "{names:?}");
    let distribution = set
        .select(Kind::IosDistribution, Some("TESTTEAM01"), None)
        .unwrap();
    assert_eq!(
        distribution.certificate.common_name,
        "Apple Distribution: Sweetpad Fixture (TESTTEAM01)"
    );
    assert_eq!(
        distribution.certificate.kinds(),
        [Kind::IosDistribution, Kind::MacAppDistribution]
    );
    let installer = set.select(Kind::MacInstaller, None, None).unwrap();
    assert!(
        installer
            .certificate
            .common_name
            .starts_with("3rd Party Mac Developer Installer")
    );
    assert!(installer.matches("Mac Installer Distribution"));
    let developer_id = set
        .select(
            Kind::DeveloperIdApplication,
            None,
            Some("Developer ID Application"),
        )
        .unwrap();
    assert!(!developer_id.is_development());
    // A certificate without Apple's extensions is read by its name.
    let fixture = set
        .select(Kind::Development, Some("TESTTEAM01"), None)
        .unwrap();
    assert_eq!(fixture.certificate.der, fixture_identity().certificate.der);
    let other = set
        .select(Kind::Development, Some("OTHERTEAM2"), None)
        .unwrap();
    assert_eq!(other.certificate.kinds(), [Kind::Development]);
    assert_eq!(set.teams(), ["OTHERTEAM2", "TESTTEAM01"]);

    let missing = set
        .select(Kind::DeveloperIdInstaller, None, None)
        .err()
        .unwrap();
    assert!(
        missing.starts_with(
            "no identity in SWEETPAD_CODE_SIGN_P12 is a valid Developer ID Installer identity; \
             it holds 'Apple Distribution: Sweetpad Fixture (TESTTEAM01)' ("
        ),
        "{missing}"
    );
    // codesign takes a name only when it picks one identity.
    let ambiguous = set.by_name("iPhone Developer").err().unwrap();
    assert!(ambiguous.contains("is ambiguous"), "{ambiguous}");
    let sha1 = other.certificate.sha1_hex();
    assert_eq!(
        set.by_name(&sha1.to_lowercase())
            .unwrap()
            .certificate
            .sha1_hex(),
        sha1
    );
    assert_eq!(set.named("Apple Development", Some("OTHERTEAM2")).len(), 1);
}

#[test]
fn passwords_go_one_per_file_or_one_for_all() {
    let files = [
        fixtures().join("identities.p12"),
        fixtures().join("identity.p12"),
    ];
    for passwords in ["sweetpad-test", "sweetpad-test:sweetpad-test"] {
        let set = identity::IdentitySet::from_files(&files, passwords, false).unwrap();
        assert_eq!(set.identities.len(), 5, "{passwords}");
        assert!(set.identities.iter().all(|i| i.key.is_none()));
    }
    let wrong = identity::IdentitySet::from_files(&files, "sweetpad-test:nope", false)
        .err()
        .unwrap();
    assert!(wrong.contains("identity.p12: "), "{wrong}");
    assert!(wrong.contains("wrong password"), "{wrong}");
    // The same identity in two files is one identity.
    let twice = [
        fixtures().join("identity.p12"),
        fixtures().join("identity-aes.p12"),
    ];
    let set = identity::IdentitySet::from_files(&twice, "sweetpad-test", false).unwrap();
    assert_eq!(set.identities.len(), 1);
}

#[test]
fn provisioning_picks_the_identity_of_the_requested_team() {
    let mut request = request(&[]);
    plist::set(
        &mut request,
        "signingCertificateIdentifier",
        Plist::String("Apple Development".into()),
    );
    let answer = |request: &Plist| {
        provisioning::answer(request, &[fixtures().join("profiles")], || {
            Ok(Some(kind_set()))
        })
    };
    plist::set(&mut request, "teamID", Plist::String("TESTTEAM01".into()));
    let inputs = answer(&request).unwrap();
    assert_eq!(
        plist::string(&inputs, "identityHash"),
        Some(fixture_identity().certificate.sha1_hex().as_str())
    );
    assert_eq!(
        plist::string(&inputs, "profileName"),
        Some("Sweetpad Fixture Wildcard")
    );
    plist::set(&mut request, "teamID", Plist::String("THIRDTEAM3".into()));
    let error = answer(&request).err().unwrap();
    assert!(
        error.starts_with("DEVELOPMENT_TEAM is THIRDTEAM3, but '"),
        "{error}"
    );
}

#[test]
fn doctor_says_what_stops_an_identity_signing() {
    let usable = apple_issued_fixture();
    let now = usable.certificate.not_before.clone();
    let wwdr_g5 = identity::Certificate::parse(include_bytes!("certs/AppleWWDRCAG5.cer")).unwrap();
    let mut unchained = apple_issued_fixture();
    unchained.certificate.issuer = wwdr_g5.subject;
    unchained.chain.clear();
    let set = identity::IdentitySet {
        identities: vec![usable, unchained],
    };
    let statuses = describe_identities(&set, &now);
    let name = "Apple Development: Sweetpad Fixture (TESTTEAM01)";
    assert_eq!(statuses[0].text, format!("{name} (development)"));
    assert!(statuses[0].usable);
    assert_eq!(
        statuses[1].text,
        format!(
            "{name} (development, issuer Apple Worldwide Developer Relations \
             Certification Authority (G5) missing)"
        )
    );
    assert!(!statuses[1].usable);
    let later = set.identities[0].certificate.not_after.clone();
    let statuses = describe_identities(&set, &later);
    assert_eq!(statuses[0].text, format!("{name} (development, expired)"));
    assert!(!statuses[0].usable);
}

#[test]
fn doctor_counts_the_profiles_something_can_sign_with() {
    let profiles = kind_profiles();
    let all = profiles.len();
    let before = "2000-01-01T00:00:00Z";
    let summary = summarize_profiles(&profiles, None, before);
    assert_eq!(summary.kinds.len(), all);
    assert!(summary.kinds.iter().all(|(_, n)| *n == 1));
    assert_eq!(
        (summary.usable, summary.expired, summary.unmatched),
        (all, 0, 0)
    );
    let summary = summarize_profiles(&profiles, Some(&kind_set()), before);
    assert_eq!(
        (summary.usable, summary.expired, summary.unmatched),
        (all, 0, 0)
    );
    let nobody = identity::IdentitySet { identities: vec![] };
    let summary = summarize_profiles(&profiles, Some(&nobody), before);
    assert_eq!(
        (summary.usable, summary.expired, summary.unmatched),
        (0, 0, all)
    );
    let summary = summarize_profiles(&profiles, Some(&kind_set()), "9999-01-01T00:00:00Z");
    assert_eq!(
        (summary.usable, summary.expired, summary.unmatched),
        (0, all, 0)
    );
}

fn kind_profiles() -> Vec<provisioning::Profile> {
    provisioning::load_profiles(&[fixtures().join("profiles-kinds")])
}

#[test]
fn profiles_are_told_apart_by_kind() {
    use provisioning::ProfileKind;
    let kinds: Vec<(String, ProfileKind)> = kind_profiles()
        .iter()
        .map(|p| {
            (
                p.name.trim_start_matches("Sweetpad Kinds ").to_string(),
                p.kind(),
            )
        })
        .collect();
    let expected = [
        ("ad-hoc", ProfileKind::AdHoc),
        ("app-store", ProfileKind::AppStore),
        ("developer-id", ProfileKind::DeveloperId),
        ("development", ProfileKind::Development),
        ("enterprise", ProfileKind::Enterprise),
        ("mac-app-store", ProfileKind::MacAppStore),
        ("mac-development", ProfileKind::MacDevelopment),
    ]
    .map(|(name, kind)| (name.to_string(), kind));
    assert_eq!(kinds, expected);
    let mac = provisioning::find_named(kind_profiles(), "Sweetpad Kinds mac-app-store").unwrap();
    assert_eq!(
        mac.application_identifier(),
        Some("TESTTEAM01.dev.sweetpad.kinds.*")
    );
}

#[test]
fn export_picks_the_profile_of_its_kind() {
    use identity::Kind;
    use provisioning::ProfileKind;
    let set = kind_set();
    let team = Some("TESTTEAM01");
    let distribution = set.select(Kind::IosDistribution, team, None).unwrap();
    let developer_id = set
        .select(Kind::DeveloperIdApplication, team, None)
        .unwrap();
    let development = set.select(Kind::Development, team, None).unwrap();
    let pick = |kind, identity, platform| {
        provisioning::pick_for_export(
            kind_profiles(),
            kind,
            identity,
            "TESTTEAM01",
            "dev.sweetpad.kinds.app",
            platform,
        )
        .map(|p| p.name)
    };
    for (kind, identity, platform, name) in [
        (ProfileKind::AppStore, distribution, "iOS", "app-store"),
        (ProfileKind::AdHoc, distribution, "iOS", "ad-hoc"),
        (ProfileKind::Enterprise, distribution, "xrOS", "enterprise"),
        (ProfileKind::Development, development, "iOS", "development"),
        (
            ProfileKind::MacAppStore,
            distribution,
            "OSX",
            "mac-app-store",
        ),
        (
            ProfileKind::DeveloperId,
            developer_id,
            "OSX",
            "developer-id",
        ),
    ] {
        assert_eq!(
            pick(kind, identity, platform).as_deref(),
            Some(format!("Sweetpad Kinds {name}").as_str()),
            "{kind:?}"
        );
    }
    // A profile without the identity's certificate, or for another
    // platform, doesn't do.
    assert_eq!(pick(ProfileKind::AppStore, development, "iOS"), None);
    assert_eq!(pick(ProfileKind::AppStore, distribution, "tvOS"), None);
}

#[test]
fn a_named_profile_must_fit_the_target() {
    let set = kind_set();
    let distribution = set
        .select(identity::Kind::IosDistribution, None, None)
        .unwrap();
    let profile = provisioning::find_named(kind_profiles(), "Sweetpad Kinds app-store").unwrap();
    provisioning::check_named(
        &profile,
        distribution,
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
    )
    .unwrap();
    let other =
        provisioning::check_named(&profile, distribution, "TESTTEAM01", "dev.sweetpad.other")
            .err()
            .unwrap();
    assert_eq!(
        other,
        "Provisioning profile \"Sweetpad Kinds app-store\" has app ID \"dev.sweetpad.kinds.*\", \
         which does not match the bundle ID \"dev.sweetpad.other\"."
    );
    let development = set
        .select(identity::Kind::Development, Some("TESTTEAM01"), None)
        .unwrap();
    let certificate = provisioning::check_named(
        &profile,
        development,
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
    )
    .err()
    .unwrap();
    assert!(
        certificate.contains("doesn't include signing certificate"),
        "{certificate}"
    );
}

#[test]
fn entitlement_values_are_checked_against_the_profile() {
    let profile = provisioning::find_named(kind_profiles(), "Sweetpad Kinds app-store").unwrap();
    let strings =
        |items: &[&str]| Plist::Array(items.iter().map(|i| Plist::String((*i).into())).collect());
    let check = |key: &str, value: Plist| {
        provisioning::check_entitlements(&profile, &[(key.to_string(), value)], "TESTTEAM01")
    };
    check(
        "com.apple.security.application-groups",
        strings(&["group.dev.sweetpad.kinds"]),
    )
    .unwrap();
    check(
        "keychain-access-groups",
        strings(&["$(AppIdentifierPrefix)dev.sweetpad.kinds.app"]),
    )
    .unwrap();
    // The profile sets the push environment, whatever the file says.
    check("aps-environment", Plist::String("development".into())).unwrap();
    let group = check(
        "com.apple.security.application-groups",
        strings(&["group.other"]),
    )
    .err()
    .unwrap();
    assert_eq!(
        group,
        "Provisioning profile \"Sweetpad Kinds app-store\" doesn't match the entitlements \
         file's value for the com.apple.security.application-groups entitlement."
    );
    let keychain = check(
        "keychain-access-groups",
        strings(&["OTHERTEAM2.dev.sweetpad.kinds"]),
    );
    assert!(keychain.is_err());
    let missing = check(
        "com.apple.developer.icloud-services",
        strings(&["CloudKit"]),
    )
    .err()
    .unwrap();
    assert!(
        missing.ends_with("doesn't include the com.apple.developer.icloud-services entitlement."),
        "{missing}"
    );
}

#[test]
fn a_developer_id_identity_gets_codesigns_designated_requirement() {
    let mut set = kind_set();
    let index = set
        .identities
        .iter()
        .position(|i| i.certificate.is(identity::Kind::DeveloperIdApplication))
        .unwrap();
    let mut identity = set.identities.swap_remove(index);
    identity.chain = [
        include_bytes!("certs/DeveloperIDG2CA.cer").as_slice(),
        include_bytes!("certs/AppleRootCA.cer"),
    ]
    .iter()
    .map(|der| identity::Certificate::parse(der).unwrap())
    .collect();
    // What Apple's codesign writes for a Developer ID Application identity,
    // read from a real signature with the team swapped for the fixture's:
    // identifier "dev.sweetpad.tool" and anchor apple generic and certificate
    // 1[field.1.2.840.113635.100.6.2.6] exists and certificate
    // leaf[field.1.2.840.113635.100.6.1.13] exists and certificate
    // leaf[subject.OU] = TESTTEAM01
    let expected = "fade0c01 000000b4 00000001 00000003 00000014 fade0c00 000000a0 00000001 \
        00000006 00000002 00000011 6465762e 73776565 74706164 2e746f6f 6c000000 00000006 \
        0000000f 00000006 0000000e 00000001 0000000a 2a864886 f7636406 02060000 00000000 \
        00000006 0000000e 00000000 0000000a 2a864886 f7636406 010d0000 00000000 0000000b \
        00000000 0000000a 7375626a 6563742e 4f550000 00000001 0000000a 54455354 5445414d \
        30310000";
    let requirements = blobs::requirements("dev.sweetpad.tool", Some(&identity)).unwrap();
    assert_eq!(
        crypto::hex(&requirements),
        expected.replace([' ', '\n'], "")
    );
}

#[test]
fn apples_authorities_are_the_ones_pinned() {
    for (der, sha256) in [
        (
            include_bytes!("certs/AppleRootCA.cer").as_slice(),
            "b0b1730ecbc7ff4505142c49f1295e6eda6bcaed7e2c68c5be91b5a11001f024",
        ),
        (
            include_bytes!("certs/AppleWWDRCAG3.cer"),
            "dcf21878c77f4198e4b4614f03d696d89c66c66008d4244e1b99161aac91601f",
        ),
        (
            include_bytes!("certs/DeveloperIDCA.cer"),
            "7afc9d01a62f03a2de9637936d4afe68090d2de18d03f29c88cfb0b1ba63587f",
        ),
        (
            include_bytes!("certs/DeveloperIDG2CA.cer"),
            "f16cd3c54c7f83cea4bf1a3e6a0819c8aaa8e4a1528fd144715f350643d2df3a",
        ),
        (
            include_bytes!("certs/AppleRootCAG3.cer"),
            "63343abfb89a6a03ebb57e9b3f5fa7be7c4f5c756f3017b3a8c488c3653e9179",
        ),
        (
            include_bytes!("certs/AppleWWDRCAG4.cer"),
            "ea4757885538dd8cb59ff4556f676087d83c85e70902c122e42c0808b5bce14c",
        ),
        (
            include_bytes!("certs/AppleWWDRCAG5.cer"),
            "53fd008278e5a595fe1e908ae9c5e5675f26243264a5a6438c023e3ce2870760",
        ),
        (
            include_bytes!("certs/AppleWWDRCAG6.cer"),
            "bdd4ed6e74691f0c2bfd01be0296197af1379e0418e2d300efa9c3bef642ca30",
        ),
    ] {
        assert_eq!(crypto::hex(&crypto::sha256(der)), sha256);
    }
}

#[test]
fn a_certificate_from_any_wwdr_authority_reaches_apples_root() {
    let pool = identity::with_authorities(&[]).unwrap();
    let rsa_root = include_bytes!("certs/AppleRootCA.cer").as_slice();
    let ecc_root = include_bytes!("certs/AppleRootCAG3.cer").as_slice();
    let expected = blobs::requirements("id", Some(&apple_issued_fixture())).unwrap();
    for (generation, root) in [
        ("G3", rsa_root),
        ("G4", rsa_root),
        ("G5", rsa_root),
        ("G6", ecc_root),
    ] {
        let authority = pool
            .iter()
            .find(|c| {
                c.organizational_unit.as_deref() == Some(generation)
                    && c.has_extension(der::oids::APPLE_WWDR_INTERMEDIATE)
            })
            .unwrap_or_else(|| panic!("WWDR {generation} isn't bundled"));
        // The fixture certificate, as if that authority had issued it and
        // its .p12 held nothing above it.
        let mut identity = fixture_identity();
        identity.certificate.issuer = authority.subject.clone();
        identity.chain = identity::chain(&identity.certificate, &pool);
        assert_eq!(identity.chain.len(), 2, "{generation}");
        assert_eq!(identity.chain[0].der, authority.der, "{generation}");
        assert_eq!(identity.root().der, root, "{generation}");
        identity.require_chain().unwrap();
        assert!(identity.is_apple_issued(), "{generation}");
        assert_eq!(identity.team(), Some("TESTTEAM01"), "{generation}");
        assert_eq!(
            blobs::requirements("id", Some(&identity)).unwrap(),
            expected,
            "{generation}"
        );
    }
}

/// The fixture app as an archive holds it, renamed for the kind profiles:
/// with a framework and a library beside its extension.
fn archived_app(dir: &TempDir) -> PathBuf {
    let app = dir.join("Payload/App.app");
    copy_tree(&fixtures().join("input/App.app"), &app);
    let info = |id: &str, executable: &str, kind: &str| {
        plist::to_xml(&Plist::Dict(vec![
            (
                "CFBundleExecutable".into(),
                Plist::String(executable.into()),
            ),
            ("CFBundleIdentifier".into(), Plist::String(id.into())),
            ("CFBundlePackageType".into(), Plist::String(kind.into())),
            ("CFBundleVersion".into(), Plist::String("1".into())),
            ("DTPlatformName".into(), Plist::String("iphoneos".into())),
        ]))
    };
    std::fs::write(
        app.join("Info.plist"),
        info("dev.sweetpad.kinds.app", "App", "APPL"),
    )
    .unwrap();
    let extension = app.join("PlugIns/Extension.appex");
    std::fs::write(
        extension.join("Info.plist"),
        info("dev.sweetpad.kinds.app.extension", "Extension", "XPC!"),
    )
    .unwrap();
    let kit = app.join("Frameworks/Kit.framework");
    std::fs::create_dir_all(&kit).unwrap();
    std::fs::copy(fixtures().join("input/Library.dylib"), kit.join("Kit")).unwrap();
    std::fs::write(
        kit.join("Info.plist"),
        info("dev.sweetpad.kinds.kit", "Kit", "FMWK"),
    )
    .unwrap();
    std::fs::copy(
        fixtures().join("input/Library.dylib"),
        app.join("Frameworks/libExtra.dylib"),
    )
    .unwrap();
    app
}

fn signature_of(path: &Path) -> blobs::Existing {
    let data = std::fs::read(path).unwrap();
    let slice = macho::split(&data).unwrap().remove(0);
    let layout = macho::Layout::parse(&slice.data).unwrap();
    blobs::read_existing(layout.signature(&slice.data).unwrap()).unwrap()
}

#[test]
fn an_export_signs_inside_out_with_the_methods_identity_and_profiles() {
    let dir = TempDir::new("resign-app-store");
    let app = archived_app(&dir);
    let set = kind_set();
    let request = resign::Request {
        app: app.clone(),
        method: resign::Method::AppStoreConnect,
        team: Some("TESTTEAM01".into()),
        manual: false,
        signing_certificate: None,
        profiles: Vec::new(),
        icloud_environment: None,
    };
    let plan = resign::plan_with(&request, &set, &kind_profiles()).unwrap();
    assert_eq!(
        plan.identity,
        "Apple Distribution: Sweetpad Fixture (TESTTEAM01)"
    );
    let steps: Vec<(String, Option<&str>)> = plan
        .steps
        .iter()
        .map(|s| {
            (
                s.path.strip_prefix(&app).unwrap().display().to_string(),
                s.profile(),
            )
        })
        .collect();
    let store = Some("Sweetpad Kinds app-store");
    assert_eq!(
        steps,
        [
            ("Frameworks/Kit.framework".to_string(), None),
            ("Frameworks/libExtra.dylib".to_string(), None),
            ("PlugIns/Extension.appex".to_string(), store),
            (String::new(), store),
        ]
    );
    resign::apply_with(&plan, &set, std::time::SystemTime::now()).unwrap();

    let main = signature_of(&app.join("App"));
    assert_eq!(main.identifier, "dev.sweetpad.kinds.app");
    let entitlements = plist::parse(main.entitlements.as_deref().unwrap()).unwrap();
    for (key, value) in [
        (
            "application-identifier",
            Plist::String("TESTTEAM01.dev.sweetpad.kinds.app".into()),
        ),
        (
            "com.apple.developer.team-identifier",
            Plist::String("TESTTEAM01".into()),
        ),
        ("get-task-allow", Plist::Bool(false)),
        ("beta-reports-active", Plist::Bool(true)),
    ] {
        assert_eq!(plist::get(&entitlements, key), Some(&value), "{key}");
    }
    assert!(app.join("embedded.mobileprovision").is_file());
    assert!(
        app.join("PlugIns/Extension.appex/embedded.mobileprovision")
            .is_file()
    );
    let extension = signature_of(&app.join("PlugIns/Extension.appex/Extension"));
    assert_eq!(extension.identifier, "dev.sweetpad.kinds.app.extension");
    assert!(extension.entitlements.is_some());
    let kit = signature_of(&app.join("Frameworks/Kit.framework/Kit"));
    assert_eq!(kit.identifier, "dev.sweetpad.kinds.kit");
    assert!(kit.entitlements.is_none());
    // The app's seal holds the extension as it is signed now.
    let resources = std::fs::read_to_string(app.join("_CodeSignature/CodeResources")).unwrap();
    assert!(resources.contains("PlugIns/Extension.appex/embedded.mobileprovision"));
    assert!(resources.contains("Frameworks/Kit.framework/_CodeSignature/CodeResources"));
}

#[test]
fn an_export_refuses_a_profile_of_another_kind_or_a_missing_one() {
    let dir = TempDir::new("resign-refusals");
    let app = archived_app(&dir);
    let set = kind_set();
    let mut request = resign::Request {
        app,
        method: resign::Method::ReleaseTesting,
        team: Some("TESTTEAM01".into()),
        manual: false,
        signing_certificate: None,
        profiles: vec![(
            "dev.sweetpad.kinds.app".into(),
            "Sweetpad Kinds app-store".into(),
        )],
        icloud_environment: None,
    };
    let named = resign::plan_with(&request, &set, &kind_profiles())
        .err()
        .unwrap();
    assert_eq!(
        named,
        "Provisioning profile \"Sweetpad Kinds app-store\" is an App Store profile, and this \
         export needs an ad hoc one"
    );
    request.profiles.clear();
    request.manual = true;
    let manual = resign::plan_with(&request, &set, &kind_profiles())
        .err()
        .unwrap();
    assert!(
        manual.contains("requires a provisioning profile"),
        "{manual}"
    );
    request.manual = false;
    let plan = resign::plan_with(&request, &set, &kind_profiles()).unwrap();
    assert_eq!(
        plan.steps.last().unwrap().profile(),
        Some("Sweetpad Kinds ad-hoc")
    );
    request.method = resign::Method::DeveloperId;
    let runtime = resign::plan_with(&request, &set, &kind_profiles())
        .err()
        .unwrap();
    assert!(
        runtime.ends_with("set ENABLE_HARDENED_RUNTIME = YES and archive again"),
        "{runtime}"
    );
    request.team = None;
    let teams = resign::plan_with(&request, &set, &kind_profiles())
        .err()
        .unwrap();
    assert!(teams.contains("names no team"), "{teams}");
}

#[test]
#[allow(clippy::too_many_lines)] // one table of entitlements per method
fn export_entitlements_move_to_the_profile_and_team() {
    let profiles = kind_profiles();
    let profile = |name: &str| {
        provisioning::find_named(profiles.clone(), &format!("Sweetpad Kinds {name}")).unwrap()
    };
    let strings =
        |items: &[&str]| Plist::Array(items.iter().map(|i| Plist::String((*i).into())).collect());
    let archived = Plist::Dict(vec![
        (
            "application-identifier".into(),
            Plist::String("OLDTEAM001.dev.sweetpad.kinds.app".into()),
        ),
        (
            "com.apple.developer.team-identifier".into(),
            Plist::String("OLDTEAM001".into()),
        ),
        ("get-task-allow".into(), Plist::Bool(true)),
        (
            "aps-environment".into(),
            Plist::String("development".into()),
        ),
        (
            "keychain-access-groups".into(),
            strings(&["OLDTEAM001.dev.sweetpad.kinds.app"]),
        ),
        (
            "com.apple.security.application-groups".into(),
            strings(&["group.dev.sweetpad.kinds"]),
        ),
    ]);
    let store = resign::transform(
        archived.clone(),
        &profile("app-store"),
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
        false,
        resign::Method::AppStoreConnect,
    )
    .unwrap();
    for (key, value) in [
        (
            "application-identifier",
            Plist::String("TESTTEAM01.dev.sweetpad.kinds.app".into()),
        ),
        (
            "com.apple.developer.team-identifier",
            Plist::String("TESTTEAM01".into()),
        ),
        ("get-task-allow", Plist::Bool(false)),
        ("aps-environment", Plist::String("production".into())),
        ("beta-reports-active", Plist::Bool(true)),
        (
            "keychain-access-groups",
            strings(&["TESTTEAM01.dev.sweetpad.kinds.app"]),
        ),
        (
            "com.apple.security.application-groups",
            strings(&["group.dev.sweetpad.kinds"]),
        ),
    ] {
        assert_eq!(plist::get(&store, key), Some(&value), "{key}");
    }
    let ad_hoc = resign::transform(
        archived.clone(),
        &profile("ad-hoc"),
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
        false,
        resign::Method::ReleaseTesting,
    )
    .unwrap();
    assert_eq!(plist::get(&ad_hoc, "beta-reports-active"), None);

    let mac = Plist::Dict(vec![
        (
            "com.apple.security.get-task-allow".into(),
            Plist::Bool(true),
        ),
        ("com.apple.security.app-sandbox".into(), Plist::Bool(true)),
    ]);
    let mac = resign::transform(
        mac,
        &profile("mac-app-store"),
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
        true,
        resign::Method::AppStoreConnect,
    )
    .unwrap();
    assert_eq!(plist::get(&mac, "com.apple.security.get-task-allow"), None);
    assert_eq!(
        plist::string(&mac, "com.apple.application-identifier"),
        Some("TESTTEAM01.dev.sweetpad.kinds.app")
    );

    let mut other = archived;
    plist::set(
        &mut other,
        "com.apple.security.application-groups",
        strings(&["group.other"]),
    );
    let refused = resign::transform(
        other,
        &profile("app-store"),
        "TESTTEAM01",
        "dev.sweetpad.kinds.app",
        false,
        resign::Method::AppStoreConnect,
    );
    assert!(
        refused
            .err()
            .unwrap()
            .contains("com.apple.security.application-groups")
    );
}

#[test]
fn an_archive_target_the_identities_cant_sign_is_signed_ad_hoc_with_its_entitlements() {
    let groups = Plist::Array(vec![Plist::String(
        "$(AppIdentifierPrefix)dev.sweetpad.fixture".into(),
    )]);
    let request = request(&[(
        "projectEntitlements",
        Plist::Dict(vec![("keychain-access-groups".into(), groups.clone())]),
    )]);
    let inputs = provisioning::archive_ad_hoc(&request, "no identity matches 'Apple Development'");
    assert_eq!(plist::string(&inputs, "identityHash"), Some("-"));
    assert_eq!(
        plist::string(&inputs, "appIdentifierPrefix"),
        Some("TESTTEAM01.")
    );
    let signed = plist::get(&inputs, "signedEntitlements").unwrap();
    assert_eq!(
        plist::string(signed, "application-identifier"),
        Some("TESTTEAM01.dev.sweetpad.fixture")
    );
    assert_eq!(plist::get(signed, "keychain-access-groups"), Some(&groups));
    assert_eq!(plist::get(signed, "get-task-allow"), None);
    let Some(Plist::Array(warnings)) = plist::get(&inputs, "warnings") else {
        panic!("no warnings");
    };
    assert_eq!(
        warnings,
        &[Plist::String(
            "signing 'dev.sweetpad.fixture' ad hoc for its export to sign again: no identity \
             matches 'Apple Development'"
                .into()
        )]
    );

    let teamless = provisioning::archive_ad_hoc(&request_without_team(), "why");
    assert_eq!(plist::get(&teamless, "appIdentifierPrefix"), None);
    let signed = plist::get(&teamless, "signedEntitlements").unwrap();
    assert_eq!(
        plist::string(signed, "application-identifier"),
        Some("dev.sweetpad.fixture")
    );
    let Some(Plist::Array(warnings)) = plist::get(&teamless, "warnings") else {
        panic!("no warnings");
    };
    assert!(
        matches!(&warnings[0], Plist::String(w) if w.ends_with("set DEVELOPMENT_TEAM so its entitlements name the team"))
    );
}

fn request_without_team() -> Plist {
    request(&[("teamID", Plist::String(String::new()))])
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
        Ok(Some(identity::IdentitySet {
            identities: vec![apple_issued_fixture()],
        }))
    })
}

#[test]
fn device_signing_does_not_provision_linux_host_macros() {
    for sdk in ["linux", "linux6.4"] {
        let request = request(&[("sdkRoot", Plist::String(sdk.into()))]);
        let inputs = provisioning::answer(&request, &[], || {
            panic!("host macros must not load the Apple identity")
        })
        .unwrap();
        assert_eq!(inputs, Plist::Dict(Vec::new()));
    }
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

/// A simulator build is signed to run locally, whatever identity it names,
/// with its entitlements in the executable's `__entitlements` section, as
/// Xcode 27's own simulator build writes them.
#[test]
fn a_simulator_build_carries_its_entitlements_in_the_executable() {
    let entitlements = Plist::Dict(vec![
        (
            "com.apple.security.application-groups".to_string(),
            Plist::Array(vec![Plist::String(
                "group.dev.sweetpad.fixture".to_string(),
            )]),
        ),
        (
            "keychain-access-groups".to_string(),
            Plist::Array(vec![Plist::String(
                "$(AppIdentifierPrefix)dev.sweetpad.fixture.shared".to_string(),
            )]),
        ),
    ]);
    let simulator = |team: &str| {
        let request = request(&[
            ("sdkRoot", Plist::String("iphonesimulator27.2".to_string())),
            (
                "entitlementsDestination",
                Plist::String("__entitlements".to_string()),
            ),
            ("projectEntitlements", entitlements.clone()),
            ("teamID", Plist::String(team.to_string())),
        ]);
        provisioning::answer(&request, &[fixtures().join("profiles")], || {
            panic!("a simulator build needs no identity")
        })
        .unwrap()
    };
    let inputs = simulator("TESTTEAM01");
    assert_eq!(plist::string(&inputs, "identityHash"), Some("-"));
    assert_eq!(
        plist::get(&inputs, "signedEntitlements"),
        Some(&Plist::Dict(Vec::new()))
    );
    let simulated = plist::get(&inputs, "simulatedEntitlements").unwrap();
    assert_eq!(
        plist::string(simulated, "application-identifier"),
        Some("TESTTEAM01.dev.sweetpad.fixture")
    );
    assert_eq!(
        plist::get(simulated, "com.apple.security.application-groups"),
        plist::get(&entitlements, "com.apple.security.application-groups")
    );
    // Swift Build puts the prefix on the keychain groups, from the inputs.
    assert_eq!(
        plist::get(simulated, "keychain-access-groups"),
        plist::get(&entitlements, "keychain-access-groups")
    );
    assert_eq!(
        plist::string(&inputs, "appIdentifierPrefix"),
        Some("TESTTEAM01.")
    );
    let teamless = simulator("");
    let simulated = plist::get(&teamless, "simulatedEntitlements").unwrap();
    assert_eq!(
        plist::string(simulated, "application-identifier"),
        Some("dev.sweetpad.fixture")
    );
    assert_eq!(plist::get(&teamless, "appIdentifierPrefix"), None);
}

/// A device build takes only a development profile that lists its device,
/// automatic or named, as Xcode does.
#[test]
fn a_device_build_needs_a_profile_listing_the_device() {
    let device = |udid: &str| ("sweetpadDeviceIdentifier", Plist::String(udid.to_string()));
    let listed = provision(&request(&[device("00008110-000000000000001e")])).unwrap();
    assert_eq!(
        plist::string(&listed, "profileName"),
        Some("Sweetpad Fixture Wildcard")
    );
    let other = "00008110-0000000000000099";
    let automatic = provision(&request(&[device(other)])).unwrap_err();
    assert_eq!(
        automatic,
        format!(
            "Provisioning profile \"Sweetpad Fixture Wildcard\" doesn't include the currently \
             selected device (identifier {other})."
        )
    );
    let manual = provision(&request(&[
        device(other),
        ("provisioningStyle", Plist::String("manual".to_string())),
        (
            "provisioningProfileSpecifier",
            Plist::String("Sweetpad Fixture Wildcard".to_string()),
        ),
    ]))
    .unwrap_err();
    assert_eq!(manual, automatic);
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

/// What Swift Build signs in a macOS app, inside out, and whether with the
/// entitlements; `regen.py` signs the same.
const MAC_STEPS: &[(&str, bool)] = &[
    ("Contents/Frameworks/libThing.dylib", false),
    ("Contents/Frameworks/Kit.framework/Versions/A", false),
    ("Contents/XPCServices/Worker.xpc", true),
    ("Contents/MacOS/Helper", false),
    ("", true),
];

/// Sign a copy of `input/Mac.app` in `dir` as Swift Build asks `codesign`
/// to: with the hardened runtime when there's an identity.
fn sign_mac(dir: &Path, name: &str, identity: Option<&Identity>) -> PathBuf {
    let app = dir.join(name);
    crate::cli::commands::archive::copy_tree(&fixtures().join("input/Mac.app"), &app).unwrap();
    for (step, entitlements) in MAC_STEPS {
        let options = Options {
            identity: identity.map_or("-".to_string(), |i| i.certificate.sha1_hex()),
            entitlements: entitlements.then(|| fixtures().join("input/mac.xcent")),
            der_entitlements: true,
            flags: if identity.is_some() {
                blobs::FLAG_RUNTIME
            } else {
                0
            },
            timestamp: Timestamp::None,
            ..Options::default()
        };
        let signer = Signer {
            options: &options,
            identity,
            time: SystemTime::now(),
        };
        let path = app.join(step);
        if path.is_dir() {
            signer.sign_bundle(&path).unwrap();
        } else {
            signer.sign_file(&path, None).unwrap();
        }
    }
    app
}

/// That every file under `ours` is `expected`'s: the same bytes, the same
/// symlinks, and Mach-O files signed the same but for the CMS.
fn assert_same_tree(ours: &Path, expected: &Path, identity: Option<&Identity>) {
    for entry in std::fs::read_dir(expected).unwrap() {
        let entry = entry.unwrap();
        let (theirs, mine) = (entry.path(), ours.join(entry.file_name()));
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() {
            assert_eq!(
                std::fs::read_link(&mine).unwrap(),
                std::fs::read_link(&theirs).unwrap()
            );
        } else if kind.is_dir() {
            assert_same_tree(&mine, &theirs, identity);
        } else {
            let data = std::fs::read(&theirs).unwrap();
            if macho::split(&data).is_ok() {
                assert_same_signature(&mine, &theirs, identity);
            } else {
                assert!(
                    std::fs::read(&mine).unwrap() == data,
                    "{} differs from Apple's",
                    mine.display()
                );
            }
        }
    }
    let count = |dir: &Path| std::fs::read_dir(dir).unwrap().count();
    assert_eq!(count(ours), count(expected), "{}", ours.display());
}

#[test]
fn signs_a_macos_app_like_codesign() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-mac");
    for (name, identity) in [("Mac.app", Some(&identity)), ("Mac.adhoc.app", None)] {
        let ours = sign_mac(&dir, name, identity);
        if let Ok(keep) = std::env::var("SWEETPAD_KEEP_SIGNED") {
            let _ = crate::cli::commands::archive::copy_tree(&ours, &Path::new(&keep).join(name));
        }
        assert_same_tree(&ours, &fixtures().join("expected").join(name), identity);
    }
    #[cfg(target_os = "macos")]
    {
        let verifier = NativeVerifier::new();
        for name in ["Mac.app", "Mac.adhoc.app"] {
            let result = verifier.verify(&dir.join(name), true);
            assert!(
                result.status.success(),
                "codesign rejects {name}: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
}

#[test]
fn signs_a_macos_resource_bundle_like_codesign() {
    let identity = fixture_identity();
    let dir = TempDir::new("sweetpad-codesign-mac-assets");
    for (name, identity) in [
        ("Assets.bundle", Some(&identity)),
        ("Assets.adhoc.bundle", None),
    ] {
        let ours = dir.join(name);
        crate::cli::commands::archive::copy_tree(&fixtures().join("input/Assets.bundle"), &ours)
            .unwrap();
        let options = Options {
            identity: identity.map_or("-".to_string(), |i| i.certificate.sha1_hex()),
            der_entitlements: true,
            ..Options::default()
        };
        let signer = Signer {
            options: &options,
            identity,
            time: SystemTime::now(),
        };
        signer.sign_bundle(&ours).unwrap();
        for file in ["CodeDirectory", "CodeRequirements", "CodeResources"] {
            let relative = Path::new("Contents/_CodeSignature").join(file);
            assert!(
                std::fs::read(ours.join(&relative)).unwrap()
                    == std::fs::read(fixtures().join("expected").join(name).join(&relative))
                        .unwrap(),
                "{name}: {file} differs from Apple's signature"
            );
        }
    }
}

#[test]
fn nested_code_is_sealed_by_its_cdhash_and_designated_requirement() {
    let expected = fixtures().join("expected");
    let (hash, requirement) =
        inspect::designated(&expected.join("Mac.adhoc.app/Contents/Frameworks/Kit.framework"))
            .unwrap();
    // A universal file's ad hoc requirement names every slice, arm64 first.
    let hashes: Vec<&str> = requirement
        .split(" or ")
        .map(|h| h.trim_start_matches("cdhash H\"").trim_end_matches('"'))
        .collect();
    assert_eq!(hashes.len(), 2, "{requirement}");
    assert_eq!(hashes[0], crypto::hex(&hash));
    let (_, requirement) =
        inspect::designated(&expected.join("Mac.app/Contents/MacOS/Helper")).unwrap();
    assert!(
        requirement.starts_with("identifier Helper and certificate root = H\""),
        "{requirement}"
    );
    let dir = TempDir::new("sweetpad-codesign-unsigned");
    std::fs::write(dir.join("script"), "#!/bin/sh\n").unwrap();
    let unsigned = inspect::designated(&dir.join("script")).unwrap_err();
    assert_eq!(unsigned, "code object is not signed at all");
}

/// Sign `app` ad hoc inside out, the app with only the sandbox entitlement,
/// as an archive of an app with ENABLE_APP_SANDBOX is signed with no
/// identity.
fn archive_sandboxed(app: &Path, entitlements: &Path) {
    let sandbox = Plist::Dict(vec![(
        "com.apple.security.app-sandbox".to_string(),
        Plist::Bool(true),
    )]);
    std::fs::write(entitlements, plist::to_xml(&sandbox)).unwrap();
    for (step, _) in MAC_STEPS {
        let options = Options {
            identity: "-".to_string(),
            entitlements: step.is_empty().then(|| entitlements.to_path_buf()),
            der_entitlements: true,
            timestamp: Timestamp::None,
            ..Options::default()
        };
        let signer = Signer {
            options: &options,
            identity: None,
            time: SystemTime::now(),
        };
        let path = app.join(step);
        if path.is_dir() {
            signer.sign_bundle(&path).unwrap();
        } else {
            signer.sign_file(&path, None).unwrap();
        }
    }
}

#[test]
fn a_mac_app_store_export_needs_the_app_sandbox() {
    let dir = TempDir::new("resign-mac-unsandboxed");
    let app = dir.join("Mac.app");
    crate::cli::commands::archive::copy_tree(&fixtures().join("input/Mac.app"), &app).unwrap();
    let path = app.join("Contents/Info.plist");
    let mut value = plist::read_info(&std::fs::read(&path).unwrap()).unwrap();
    plist::set(
        &mut value,
        "CFBundleIdentifier",
        Plist::String("dev.sweetpad.kinds.mac".into()),
    );
    std::fs::write(&path, plist::to_xml(&value)).unwrap();
    let request = resign::Request {
        app: app.clone(),
        method: resign::Method::AppStoreConnect,
        team: Some("TESTTEAM01".into()),
        manual: false,
        signing_certificate: None,
        profiles: Vec::new(),
        icloud_environment: None,
    };
    let error = resign::plan_with(&request, &kind_set(), &kind_profiles())
        .err()
        .unwrap();
    assert!(error.contains("ENABLE_APP_SANDBOX = YES"), "{error}");
}

#[test]
fn a_macos_export_signs_inside_out_with_the_mac_app_store_profile() {
    let dir = TempDir::new("resign-mac-app-store");
    let app = dir.join("Mac.app");
    crate::cli::commands::archive::copy_tree(&fixtures().join("input/Mac.app"), &app).unwrap();
    for (info, id) in [
        ("Contents/Info.plist", "dev.sweetpad.kinds.mac"),
        (
            "Contents/XPCServices/Worker.xpc/Contents/Info.plist",
            "dev.sweetpad.kinds.mac.worker",
        ),
    ] {
        let path = app.join(info);
        let mut value = plist::read_info(&std::fs::read(&path).unwrap()).unwrap();
        plist::set(&mut value, "CFBundleIdentifier", Plist::String(id.into()));
        std::fs::write(&path, plist::to_xml(&value)).unwrap();
    }
    archive_sandboxed(&app, &dir.join("sandbox.xcent"));
    let set = kind_set();
    let request = resign::Request {
        app: app.clone(),
        method: resign::Method::AppStoreConnect,
        team: Some("TESTTEAM01".into()),
        manual: false,
        signing_certificate: None,
        profiles: Vec::new(),
        icloud_environment: None,
    };
    let plan = resign::plan_with(&request, &set, &kind_profiles()).unwrap();
    let steps: Vec<(String, Option<&str>)> = plan
        .steps
        .iter()
        .map(|s| {
            (
                s.path.strip_prefix(&app).unwrap().display().to_string(),
                s.profile(),
            )
        })
        .collect();
    assert_eq!(
        steps,
        [
            ("Contents/Frameworks/Kit.framework".to_string(), None),
            ("Contents/Frameworks/libThing.dylib".to_string(), None),
            ("Contents/XPCServices/Worker.xpc".to_string(), None),
            ("Contents/MacOS/Helper".to_string(), None),
            (String::new(), Some("Sweetpad Kinds mac-app-store")),
        ]
    );
    resign::apply_with(&plan, &set, std::time::SystemTime::now()).unwrap();
    assert!(app.join("Contents/embedded.provisionprofile").is_file());
    let main = signature_of(&app.join("Contents/MacOS/Mac"));
    assert_eq!(main.identifier, "dev.sweetpad.kinds.mac");
    let entitlements = plist::parse(main.entitlements.as_deref().unwrap()).unwrap();
    assert_eq!(
        plist::get(&entitlements, "com.apple.application-identifier"),
        Some(&Plist::String("TESTTEAM01.dev.sweetpad.kinds.mac".into()))
    );
    assert_eq!(
        plist::get(&entitlements, "com.apple.security.get-task-allow"),
        None
    );
    let kit = signature_of(&app.join("Contents/Frameworks/Kit.framework/Versions/A/Kit"));
    assert_eq!(kit.identifier, "dev.sweetpad.kit");
    let resources =
        std::fs::read_to_string(app.join("Contents/_CodeSignature/CodeResources")).unwrap();
    for sealed in [
        "Frameworks/Kit.framework",
        "XPCServices/Worker.xpc",
        "MacOS/Helper",
        "embedded.provisionprofile",
    ] {
        assert!(
            resources.contains(&format!("<key>{sealed}</key>")),
            "{sealed}"
        );
    }
}

fn developer_id_request(app: PathBuf) -> resign::Request {
    resign::Request {
        app,
        method: resign::Method::DeveloperId,
        team: Some("TESTTEAM01".into()),
        manual: false,
        signing_certificate: None,
        profiles: Vec::new(),
        icloud_environment: None,
    }
}

#[test]
fn a_developer_id_export_embeds_no_profile_for_entitlements_any_app_may_claim() {
    let dir = TempDir::new("resign-developer-id");
    let identity = fixture_identity();
    let app = sign_mac(&dir, "Mac.app", Some(&identity));
    let plan =
        resign::plan_with(&developer_id_request(app), &kind_set(), &kind_profiles()).unwrap();
    assert!(plan.identity.starts_with("Developer ID Application"));
    let main = plan.steps.last().unwrap();
    assert_eq!(main.profile(), None);
    let entitlements = main.entitlements.as_ref().unwrap();
    assert_eq!(
        plist::get(entitlements, "com.apple.security.app-sandbox"),
        Some(&Plist::Bool(true))
    );
    assert!(plist::get(entitlements, "com.apple.security.application-groups").is_some());
    for key in [
        "com.apple.application-identifier",
        "com.apple.developer.team-identifier",
        "com.apple.security.get-task-allow",
    ] {
        assert_eq!(plist::get(entitlements, key), None, "{key}");
    }
}

#[test]
fn a_developer_id_export_needs_the_hardened_runtime() {
    let dir = TempDir::new("resign-developer-id-runtime");
    let app = sign_mac(&dir, "Mac.app", None);
    let error = resign::plan_with(&developer_id_request(app), &kind_set(), &kind_profiles())
        .err()
        .unwrap();
    assert!(error.contains("ENABLE_HARDENED_RUNTIME = YES"), "{error}");
}

#[test]
fn only_capabilities_apple_grants_need_a_developer_id_profile() {
    let entitlements = |key: &str, value: Plist| Plist::Dict(vec![(key.to_string(), value)]);
    let groups = |group: &str| Plist::Array(vec![Plist::String(group.to_string())]);
    for (key, value, restricted) in [
        ("com.apple.security.app-sandbox", Plist::Bool(true), false),
        (
            "com.apple.security.get-task-allow",
            Plist::Bool(true),
            false,
        ),
        (
            "com.apple.application-identifier",
            Plist::String("TESTTEAM01.dev.app".into()),
            false,
        ),
        (
            "com.apple.security.application-groups",
            groups("TESTTEAM01.dev.app"),
            false,
        ),
        (
            "com.apple.security.application-groups",
            groups("group.dev.app"),
            true,
        ),
        (
            "com.apple.developer.aps-environment",
            Plist::String("production".into()),
            true,
        ),
        ("keychain-access-groups", groups("TESTTEAM01.dev.app"), true),
    ] {
        assert_eq!(
            resign::restricted(&entitlements(key, value)),
            restricted,
            "{key}"
        );
    }
}
