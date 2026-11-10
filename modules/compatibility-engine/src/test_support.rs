//! Fixtures shared by the crate's tests.
//!
//! Every bundle is built from a literal `Info.plist` and explicit Mach-O bytes
//! inside a temporary directory: the tests never depend on a real `.app` being
//! installed on the host, and the load commands are written the way a linker
//! writes them — a fixed structure, the command's payload, then padding to a
//! four-byte boundary.
//!
//! The inspector keeps its own fixtures private, so this module repeats the few
//! bytes a fixture needs instead of reaching into another crate's tests.

use std::fs;
use std::path::{Path, PathBuf};

/// `CPU_TYPE_X86_64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_X86`.
pub(crate) const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// `CPU_TYPE_ARM64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_ARM`.
pub(crate) const CPU_TYPE_ARM64: u32 = 0x0100_000c;

/// `LC_LOAD_DYLIB`: a library the binary needs at launch.
const LC_LOAD_DYLIB: u32 = 0x0c;
/// `LC_BUILD_VERSION`: the minimum OS declaration that replaced
/// `LC_VERSION_MIN_MACOSX`.
const LC_BUILD_VERSION: u32 = 0x32;
/// `PLATFORM_MACOS`, the only platform that describes a macOS requirement.
pub(crate) const PLATFORM_MACOS: u32 = 1;

/// Size of `dylib_command`, followed by the library path.
const DYLIB_COMMAND_LENGTH: u32 = 24;
/// Size of `build_version_command`.
const BUILD_VERSION_COMMAND_LENGTH: u32 = 24;
/// Offset of `ncmds` in `mach_header_64`.
const NCMDS_OFFSET: usize = 16;
/// Offset of `sizeofcmds`, next to `ncmds`.
const SIZEOFCMDS_OFFSET: usize = 20;
/// Size of one `fat_arch` entry.
const FAT_ARCH_LENGTH: usize = 20;

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

/// Writes the bundle's `Contents/Info.plist` as an XML property list.
pub(crate) fn write_xml_plist(bundle: &Path, xml: &str) {
    write_file(&bundle.join("Contents").join("Info.plist"), xml.as_bytes());
}

/// A thin 64-bit little-endian Mach-O header for `cputype` carrying `commands`,
/// with `ncmds` and `sizeofcmds` computed from them.
pub(crate) fn thin_macho(cputype: u32, commands: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = vec![0xcf, 0xfa, 0xed, 0xfe];
    bytes.extend_from_slice(&cputype.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes()); // cpusubtype: CPU_SUBTYPE_ANY
    bytes.extend_from_slice(&2u32.to_le_bytes()); // filetype: MH_EXECUTE
    bytes.extend_from_slice(&0u32.to_le_bytes()); // ncmds, filled in below
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sizeofcmds, filled in below
    bytes.extend_from_slice(&0u32.to_le_bytes()); // flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // reserved
    let sizeofcmds: usize = commands.iter().map(Vec::len).sum();
    bytes[NCMDS_OFFSET..SIZEOFCMDS_OFFSET].copy_from_slice(&(commands.len() as u32).to_le_bytes());
    bytes[SIZEOFCMDS_OFFSET..SIZEOFCMDS_OFFSET + 4]
        .copy_from_slice(&(sizeofcmds as u32).to_le_bytes());
    for command in commands {
        bytes.extend_from_slice(command);
    }
    bytes
}

/// A universal ("fat") binary holding one slice per entry of `slices`, in that
/// order, behind a big-endian 32-bit `fat_arch` table, which is what `lipo`
/// writes by default. Each slice is a thin binary carrying the commands paired
/// with its CPU type.
pub(crate) fn fat_macho(slices: &[(u32, Vec<Vec<u8>>)]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = slices
        .iter()
        .map(|(cputype, commands)| (*cputype, thin_macho(*cputype, commands)))
        .collect();
    let table_length = 8 + FAT_ARCH_LENGTH * images.len();
    let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe];
    bytes.extend_from_slice(&(images.len() as u32).to_be_bytes());
    let mut offset = table_length as u32;
    for (cputype, image) in &images {
        bytes.extend_from_slice(&cputype.to_be_bytes());
        bytes.extend_from_slice(&3u32.to_be_bytes()); // cpusubtype
        bytes.extend_from_slice(&offset.to_be_bytes());
        bytes.extend_from_slice(&(image.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&0u32.to_be_bytes()); // align: 2^0
        offset += image.len() as u32;
    }
    for (_, image) in &images {
        bytes.extend_from_slice(image);
    }
    bytes
}

/// An `LC_LOAD_DYLIB` command naming `name`.
pub(crate) fn dylib_command(name: &str) -> Vec<u8> {
    let mut text = name.as_bytes().to_vec();
    text.push(0);
    let unpadded = DYLIB_COMMAND_LENGTH as usize + text.len();
    let cmdsize = unpadded.div_ceil(4) * 4;
    let mut bytes = LC_LOAD_DYLIB.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(cmdsize as u32).to_le_bytes());
    bytes.extend_from_slice(&DYLIB_COMMAND_LENGTH.to_le_bytes()); // dylib.name.offset
    bytes.extend_from_slice(&0u32.to_le_bytes()); // timestamp
    bytes.extend_from_slice(&0u32.to_le_bytes()); // current_version
    bytes.extend_from_slice(&0u32.to_le_bytes()); // compatibility_version
    bytes.extend_from_slice(&text);
    bytes.resize(cmdsize, 0);
    bytes
}

/// An `LC_BUILD_VERSION` command declaring `minos` for `platform`.
pub(crate) fn build_version_command(platform: u32, minos: u32) -> Vec<u8> {
    let mut bytes = LC_BUILD_VERSION.to_le_bytes().to_vec();
    bytes.extend_from_slice(&BUILD_VERSION_COMMAND_LENGTH.to_le_bytes());
    bytes.extend_from_slice(&platform.to_le_bytes());
    bytes.extend_from_slice(&minos.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sdk
    bytes.extend_from_slice(&0u32.to_le_bytes()); // ntools
    bytes
}

/// Packs a version the way Mach-O does: 16 bits of major, 8 of minor, 8 of
/// patch.
pub(crate) fn packed_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 16) | (minor << 8) | patch
}

/// An empty temporary directory holding a bundle directory called `name`.
pub(crate) fn app_named(name: &str) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("create the temporary directory");
    let bundle = directory.path().join(name);
    create_dir(&bundle);
    (directory, bundle)
}

/// A well-formed `Info.plist` for the executable `Test`, declaring
/// `LSMinimumSystemVersion` only when `minimum` is given.
pub(crate) fn info_plist(minimum: Option<&str>) -> String {
    let minimum = match minimum {
        Some(value) => format!("\t<key>LSMinimumSystemVersion</key>\n\t<string>{value}</string>\n"),
        None => String::new(),
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleIdentifier</key>
	<string>com.example.test</string>
	<key>CFBundleName</key>
	<string>Test</string>
	<key>CFBundleExecutable</key>
	<string>Test</string>
{minimum}</dict>
</plist>
"#
    )
}

/// Writes `bytes` as the bundle's declared executable and its `Info.plist`,
/// which declares `minimum` as `LSMinimumSystemVersion` when given.
pub(crate) fn write_app(bundle: &Path, bytes: &[u8], minimum: Option<&str>) {
    write_xml_plist(bundle, &info_plist(minimum));
    write_file(&bundle.join("Contents").join("MacOS").join("Test"), bytes);
}
