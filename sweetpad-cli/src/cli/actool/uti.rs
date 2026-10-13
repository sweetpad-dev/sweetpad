//! The type identifier a data set's file gets when its `Contents.json`
//! names none: the one macOS's type database gives its extension, which
//! `NSDataAsset.typeIdentifier` reports.

/// The type identifiers of the extensions Xcode's actool knows, as it
/// gives them.
const KNOWN: &[(&str, &str)] = &[
    ("aiff", "public.aiff-audio"),
    ("bin", "com.apple.macbinary-archive"),
    ("bmp", "com.microsoft.bmp"),
    ("bundle", "com.apple.generic-bundle"),
    ("caf", "com.apple.coreaudio-format"),
    ("css", "public.css"),
    ("csv", "public.comma-separated-values-text"),
    ("doc", "com.microsoft.word.doc"),
    ("geojson", "public.geojson"),
    ("gif", "com.compuserve.gif"),
    ("gz", "org.gnu.gnu-zip-archive"),
    ("heic", "public.heic"),
    ("htm", "public.html"),
    ("html", "public.html"),
    ("ico", "com.microsoft.ico"),
    ("jpeg", "public.jpeg"),
    ("jpg", "public.jpeg"),
    ("js", "com.netscape.javascript-source"),
    ("json", "public.json"),
    ("m4a", "com.apple.m4a-audio"),
    ("md", "net.daringfireball.markdown"),
    ("mlmodel", "com.apple.coreml.model"),
    ("mov", "com.apple.quicktime-movie"),
    ("mp3", "public.mp3"),
    ("mp4", "public.mpeg-4"),
    ("otf", "public.opentype-font"),
    ("pdf", "com.adobe.pdf"),
    ("plist", "com.apple.property-list"),
    ("png", "public.png"),
    ("rtf", "public.rtf"),
    ("scn", "com.apple.scenekit.scene"),
    ("strings", "com.apple.xcode.strings-text"),
    ("svg", "public.svg-image"),
    ("tiff", "public.tiff"),
    ("tsv", "public.tab-separated-values-text"),
    ("ttf", "public.truetype-ttf-font"),
    ("txt", "public.plain-text"),
    ("usdz", "com.pixar.universal-scene-description-mobile"),
    ("wav", "com.microsoft.waveform-audio"),
    ("webp", "org.webmproject.webp"),
    ("xml", "public.xml"),
    ("yaml", "public.yaml"),
    ("yml", "public.yaml"),
    ("zip", "public.zip-archive"),
];

/// The type identifier of the file `filename`: a known extension's, or
/// else the dynamic one macOS makes up for it, `dyn.a` and the extension
/// tag `1=<extension>` in its base-32 alphabet.
pub(super) fn of(filename: &str) -> String {
    let extension = std::path::Path::new(filename)
        .extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lower = extension.to_ascii_lowercase();
    if let Some((_, known)) = KNOWN.iter().find(|(e, _)| *e == lower) {
        return (*known).to_string();
    }
    format!("dyn.a{}", base32(format!("1={extension}").as_bytes()))
}

/// The bits of `data`, five at a time from the top, in the alphabet with no
/// i, j, l or o; the last group padded with zeros.
fn base32(data: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghkmnpqrstuvwxyz0123456789";
    let mut out = String::new();
    let (mut bits, mut count) = (0u32, 0);
    for &byte in data {
        bits = (bits << 8) | u32::from(byte);
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(char::from(ALPHABET[((bits >> count) & 31) as usize]));
        }
    }
    if count > 0 {
        out.push(char::from(ALPHABET[((bits << (5 - count)) & 31) as usize]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_known_extension_gets_its_type() {
        assert_eq!(of("config.json"), "public.json");
        assert_eq!(of("notes.TXT"), "public.plain-text");
    }

    #[test]
    fn any_other_gets_the_dynamic_type_xcode_gives_it() {
        assert_eq!(of("a.foo"), "dyn.age80q55t");
        assert_eq!(of("model.tflite"), "dyn.age81k3xqrf4gn");
        assert_eq!(of("font.woff2"), "dyn.age81s55gq23a");
        assert_eq!(of("noextension"), "dyn.age8u");
    }
}
