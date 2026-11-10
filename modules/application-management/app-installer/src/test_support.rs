//! Fixtures shared by the crate's tests.
//!
//! Every fixture is built from handcrafted bytes inside a temporary directory: a
//! property list written from a literal, a Mach-O header assembled field by
//! field and an archive written entry by entry. No test depends on a real `.app`
//! bundle, a ZIP file or a macOS file being installed on the host.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// `CPU_TYPE_X86_64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_X86`.
pub(crate) const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// `CPU_TYPE_ARM64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_ARM`.
pub(crate) const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// Writes `bytes` to `path`, creating the parent directories it needs.
pub(crate) fn write_file(path: &Path, bytes: &[u8]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create the fixture directory");
    }
    fs::write(path, bytes).expect("write the fixture file");
}

/// Creates `path` and every missing parent as a directory.
pub(crate) fn create_dir(path: &Path) {
    fs::create_dir_all(path).expect("create the fixture directory");
}

/// A thin 64-bit little-endian Mach-O header for `cputype`, with no load
/// commands. The magic bytes are those a macOS binary really starts with.
pub(crate) fn thin_macho(cputype: u32) -> Vec<u8> {
    let mut bytes = vec![0xcf, 0xfa, 0xed, 0xfe];
    bytes.extend_from_slice(&cputype.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes()); // cpusubtype: CPU_SUBTYPE_ANY
    bytes.extend_from_slice(&2u32.to_le_bytes()); // filetype: MH_EXECUTE
    bytes.extend_from_slice(&0u32.to_le_bytes()); // ncmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    bytes
}

/// A universal Mach-O holding one 64-bit slice per entry of `cputypes`, behind a
/// big-endian 32-bit `fat_arch` table, which is what `lipo` writes by default.
pub(crate) fn fat_macho(cputypes: &[u32]) -> Vec<u8> {
    let slices: Vec<Vec<u8>> = cputypes.iter().map(|kind| thin_macho(*kind)).collect();
    let table_length = 8 + 20 * cputypes.len();

    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe];
    bytes.extend_from_slice(&(cputypes.len() as u32).to_be_bytes());
    let mut offset = table_length as u32;
    for (cputype, slice) in cputypes.iter().zip(&slices) {
        bytes.extend_from_slice(&cputype.to_be_bytes());
        bytes.extend_from_slice(&3u32.to_be_bytes()); // cpusubtype: CPU_SUBTYPE_ANY
        bytes.extend_from_slice(&offset.to_be_bytes());
        bytes.extend_from_slice(&(slice.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes()); // align: 2^0
        offset += slice.len() as u32;
    }
    for slice in slices {
        bytes.extend_from_slice(&slice);
    }
    bytes
}

/// An XML `Info.plist` declaring the keys the installer reads.
///
/// A key that is `None` is omitted, which is how the fixtures exercise a bundle
/// the inspector must refuse.
pub(crate) fn plist(
    bundle_id: Option<&str>,
    name: &str,
    version: Option<&str>,
    executable: &str,
    icon: Option<&str>,
) -> String {
    let mut keys = String::new();
    if let Some(bundle_id) = bundle_id {
        keys.push_str(&format!(
            "\t<key>CFBundleIdentifier</key>\n\t<string>{bundle_id}</string>\n"
        ));
    }
    keys.push_str(&format!(
        "\t<key>CFBundleName</key>\n\t<string>{name}</string>\n"
    ));
    if let Some(version) = version {
        keys.push_str(&format!(
            "\t<key>CFBundleVersion</key>\n\t<string>{version}</string>\n"
        ));
    }
    keys.push_str(&format!(
        "\t<key>CFBundleExecutable</key>\n\t<string>{executable}</string>\n"
    ));
    if let Some(icon) = icon {
        keys.push_str(&format!(
            "\t<key>CFBundleIconFile</key>\n\t<string>{icon}</string>\n"
        ));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <plist version=\"1.0\">\n<dict>\n{keys}</dict>\n</plist>\n"
    )
}

/// The `Info.plist` of [`example_bundle`].
pub(crate) fn example_plist() -> String {
    plist(
        Some("com.example.example"),
        "Example",
        Some("1.2.3"),
        "Example",
        Some("Example.icns"),
    )
}

/// Writes `parent/<name>` as a bundle: `Contents/Info.plist` from `plist` and a
/// thin x86_64 Mach-O executable at `Contents/MacOS/<executable>`.
pub(crate) fn write_bundle(parent: &Path, name: &str, executable: &str, plist: &str) -> PathBuf {
    let bundle = parent.join(name);
    write_file(
        &bundle.join("Contents").join("Info.plist"),
        plist.as_bytes(),
    );
    write_file(
        &bundle.join("Contents").join("MacOS").join(executable),
        &thin_macho(CPU_TYPE_X86_64),
    );
    bundle
}

/// A ready-to-install bundle under `parent`: identifier `com.example.example`,
/// name `Example`, version `1.2.3`, the executable `Example` and the icon
/// `Contents/Resources/Example.icns`.
pub(crate) fn example_bundle(parent: &Path) -> PathBuf {
    let bundle = write_bundle(parent, "Example.app", "Example", &example_plist());
    write_file(
        &bundle
            .join("Contents")
            .join("Resources")
            .join("Example.icns"),
        b"icns",
    );
    bundle
}

/// One entry of an archive fixture.
pub(crate) fn entry(name: &str, bytes: &[u8]) -> (String, Vec<u8>) {
    (name.to_string(), bytes.to_vec())
}

/// The entries of a complete bundle named `name` with the executable
/// `executable`, as an archive would hold them.
pub(crate) fn zip_bundle(name: &str, executable: &str, plist: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        entry(&format!("{name}/Contents/Info.plist"), plist.as_bytes()),
        entry(
            &format!("{name}/Contents/MacOS/{executable}"),
            &thin_macho(CPU_TYPE_X86_64),
        ),
    ]
}

/// Writes a stored-entry archive holding `entries` as regular files.
pub(crate) fn write_zip(path: &Path, entries: &[(String, Vec<u8>)]) {
    let mut writer = writer(path);
    for (name, bytes) in entries {
        write_file_entry(&mut writer, name, bytes);
    }
    writer.finish().expect("finish the archive");
}

/// Writes `entries` as regular files and then one symbolic link entry.
pub(crate) fn write_zip_with_symlink(
    path: &Path,
    entries: &[(String, Vec<u8>)],
    link: (&str, &str),
) {
    let mut writer = writer(path);
    for (name, bytes) in entries {
        write_file_entry(&mut writer, name, bytes);
    }
    writer
        .add_symlink(link.0, link.1, SimpleFileOptions::default())
        .expect("write the symbolic link entry");
    writer.finish().expect("finish the archive");
}

/// An archive writer over a fresh file at `path`.
fn writer(path: &Path) -> ZipWriter<fs::File> {
    if let Some(parent) = path.parent() {
        create_dir(parent);
    }
    ZipWriter::new(fs::File::create(path).expect("create the archive"))
}

/// Writes one stored regular-file entry.
fn write_file_entry(writer: &mut ZipWriter<fs::File>, name: &str, bytes: &[u8]) {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    writer.start_file(name, options).expect("write the entry");
    writer.write_all(bytes).expect("write the entry body");
}
