//! Writing a product archive's XAR container, as `productbuild` does: the
//! header, the zlib table of contents, and a heap holding the SHA-1 of that
//! table, the installer identity's RSA and CMS signatures over it, then
//! each file. The component's files sit in a `<id>.pkg` folder, the
//! Distribution beside it.

use std::fmt::Write as _;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::time::SystemTime;

use base64::Engine as _;

use crate::cli::codesign::installer::Installer;
use crate::cli::codesign::{hex, sha1, time};

/// The room `productbuild` sets aside for the CMS signature, and for one
/// that carries a timestamp.
const CMS_RESERVE: usize = 6144;
const TIMESTAMPED_CMS_RESERVE: usize = 9216;

fn cms_reserve(signer: &Installer) -> usize {
    if signer.timestamps() {
        TIMESTAMPED_CMS_RESERVE
    } else {
        CMS_RESERVE
    }
}

/// The component package's files and the product's Distribution.
pub(super) struct Product<'a> {
    /// The component's folder, `<id>.pkg`.
    pub component: &'a str,
    pub bom: &'a [u8],
    /// The gzip payload on disk, with its SHA-1 and length.
    pub payload: &'a Path,
    pub payload_sha1: [u8; 20],
    pub payload_length: u64,
    pub package_info: &'a [u8],
    pub distribution: &'a [u8],
}

/// A file of the heap: its bytes as stored and what describes them.
struct Stored {
    name: &'static str,
    archived: Vec<u8>,
    extracted_sha1: [u8; 20],
    extracted_length: u64,
}

impl Stored {
    fn compressed(name: &'static str, data: &[u8]) -> Result<Self, String> {
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(data)
            .map_err(|e| format!("couldn't compress {name}: {e}"))?;
        Ok(Self {
            name,
            archived: encoder
                .finish()
                .map_err(|e| format!("couldn't compress {name}: {e}"))?,
            extracted_sha1: sha1(data),
            extracted_length: data.len() as u64,
        })
    }

    fn data(&self, offset: u64) -> String {
        format!(
            "<data>\n<archived-checksum style=\"sha1\">{}</archived-checksum>\n\
             <extracted-checksum style=\"sha1\">{}</extracted-checksum>\n\
             <encoding style=\"application/x-gzip\"/>\n<size>{}</size>\n\
             <offset>{offset}</offset>\n<length>{}</length>\n</data>\n",
            hex(&sha1(&self.archived)),
            hex(&self.extracted_sha1),
            self.extracted_length,
            self.archived.len()
        )
    }
}

/// Write `product` to `out`, signed by `signer` when there is one.
pub(super) fn write(
    out: &Path,
    product: &Product<'_>,
    signer: Option<&Installer>,
    time: SystemTime,
) -> Result<(), String> {
    let bom = Stored::compressed("Bom", product.bom)?;
    let package_info = Stored::compressed("PackageInfo", product.package_info)?;
    let distribution = Stored::compressed("Distribution", product.distribution)?;

    let signature_size = signer.map_or(0, Installer::signature_size);
    let toc = table_of_contents(product, [&bom, &package_info, &distribution], signer, time);

    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(toc.as_bytes())
        .map_err(|e| format!("couldn't compress the table of contents: {e}"))?;
    let compressed = encoder
        .finish()
        .map_err(|e| format!("couldn't compress the table of contents: {e}"))?;
    let checksum = sha1(&compressed);
    let mut signatures = Vec::new();
    if let Some(signer) = signer {
        let rsa = signer.sign(&checksum)?;
        if rsa.len() != signature_size {
            return Err(format!(
                "the installer signature is {} bytes, not the {signature_size} its key gives",
                rsa.len()
            ));
        }
        let mut cms = signer.cms(&checksum, time)?;
        let reserve = cms_reserve(signer);
        if cms.len() > reserve {
            return Err(format!(
                "the CMS signature is {} bytes, more than the {reserve} set aside for it",
                cms.len()
            ));
        }
        cms.resize(reserve, 0);
        signatures = [rsa, cms].concat();
    }

    let failed = |e: std::io::Error| format!("couldn't write {}: {e}", out.display());
    let mut file = BufWriter::new(File::create(out).map_err(failed)?);
    let mut header = b"xar!".to_vec();
    header.extend_from_slice(&28u16.to_be_bytes());
    header.extend_from_slice(&1u16.to_be_bytes());
    header.extend_from_slice(&(compressed.len() as u64).to_be_bytes());
    header.extend_from_slice(&(toc.len() as u64).to_be_bytes());
    header.extend_from_slice(&1u32.to_be_bytes());
    for part in [
        &header,
        &compressed,
        &checksum[..],
        &signatures,
        &bom.archived,
    ] {
        file.write_all(part).map_err(failed)?;
    }
    let mut payload = File::open(product.payload)
        .map_err(|e| format!("couldn't read {}: {e}", product.payload.display()))?;
    let copied = std::io::copy(&mut payload, &mut file).map_err(failed)?;
    if copied != product.payload_length {
        return Err(format!(
            "{} changed while the package was written",
            product.payload.display()
        ));
    }
    for part in [&package_info.archived, &distribution.archived] {
        file.write_all(part).map_err(failed)?;
    }
    file.flush().map_err(failed)
}

/// The table of contents: the checksum, the signatures, then the files in
/// the order the heap holds them.
fn table_of_contents(
    product: &Product<'_>,
    [bom, package_info, distribution]: [&Stored; 3],
    signer: Option<&Installer>,
    time: SystemTime,
) -> String {
    let signature_size = signer.map_or(0, Installer::signature_size);
    let mut offset = 20 + signer.map_or(0, |s| (signature_size + cms_reserve(s)) as u64);
    let mut toc = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xar>\n<toc>\n\
         <checksum style=\"sha1\">\n<size>20</size>\n<offset>0</offset>\n</checksum>\n",
    );
    let created = time::iso8601(time);
    let _ = writeln!(
        toc,
        "<creation-time>{}</creation-time>",
        created.trim_end_matches('Z')
    );
    if let Some(signer) = signer {
        let certificates = key_info(&signer.certificates());
        let _ = write!(
            toc,
            "<signature style=\"RSA\">\n<offset>20</offset>\n<size>{signature_size}</size>\n\
             {certificates}</signature>\n\
             <x-signature style=\"CMS\">\n<offset>{}</offset>\n<size>{}</size>\n\
             {certificates}</x-signature>\n",
            20 + signature_size,
            cms_reserve(signer)
        );
    }
    let _ = write!(
        toc,
        "<file id=\"1\">\n<name>{}</name>\n<type>directory</type>\n<inode>0</inode>\n\
         <deviceno>0</deviceno>\n<mode>0700</mode>\n<uid>0</uid>\n<user>root</user>\n\
         <gid>0</gid>\n<group>wheel</group>\n<atime>1970-01-01T00:00:00Z</atime>\n\
         <mtime>1970-01-01T00:00:00Z</mtime>\n<ctime>1970-01-01T00:00:00Z</ctime>\n",
        crate::cli::xml_plist::escape(product.component)
    );
    let _ = write!(
        toc,
        "<file id=\"2\">\n<name>{}</name>\n<type>file</type>\n{}</file>\n",
        bom.name,
        bom.data(offset)
    );
    offset += bom.archived.len() as u64;
    let payload_sha1 = hex(&product.payload_sha1);
    let _ = write!(
        toc,
        "<file id=\"3\">\n<name>Payload</name>\n<type>file</type>\n<data>\n\
         <archived-checksum style=\"sha1\">{payload_sha1}</archived-checksum>\n\
         <extracted-checksum style=\"sha1\">{payload_sha1}</extracted-checksum>\n\
         <encoding style=\"application/octet-stream\"/>\n<size>{length}</size>\n\
         <offset>{offset}</offset>\n<length>{length}</length>\n</data>\n</file>\n",
        length = product.payload_length
    );
    offset += product.payload_length;
    let _ = write!(
        toc,
        "<file id=\"4\">\n<name>{}</name>\n<type>file</type>\n{}</file>\n</file>\n",
        package_info.name,
        package_info.data(offset)
    );
    offset += package_info.archived.len() as u64;
    let _ = write!(
        toc,
        "<file id=\"5\">\n<name>{}</name>\n<type>file</type>\n{}</file>\n</toc>\n</xar>\n",
        distribution.name,
        distribution.data(offset)
    );
    toc
}

/// The certificates of a signature, base64 in 72-character lines as
/// `productbuild` writes them.
fn key_info(certificates: &[&[u8]]) -> String {
    let mut out =
        String::from("<KeyInfo xmlns=\"http://www.w3.org/2000/09/xmldsig#\">\n<X509Data>\n");
    for certificate in certificates {
        let encoded = base64::engine::general_purpose::STANDARD.encode(certificate);
        let lines: Vec<&str> = encoded
            .as_bytes()
            .chunks(72)
            .map(|line| std::str::from_utf8(line).unwrap_or_default())
            .collect();
        let _ = writeln!(
            out,
            "<X509Certificate>{}</X509Certificate>",
            lines.join("\r\n")
        );
    }
    out.push_str("</X509Data>\n</KeyInfo>\n");
    out
}
