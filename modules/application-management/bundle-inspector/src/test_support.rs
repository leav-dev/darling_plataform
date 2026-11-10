//! Fixtures shared by the crate's tests.
//!
//! Every fixture is a property list written from a literal and a Mach-O header
//! built from explicit bytes inside a temporary directory. The tests never
//! depend on a real `.app` bundle being installed on the host.
//!
//! Load commands are built the way a linker writes them: a fixed structure
//! followed by the command's own payload, padded so the next command starts on a
//! four-byte boundary.

use std::fs;
use std::path::{Path, PathBuf};

use plist::{Dictionary, Value};

/// `CPU_TYPE_I386`; 32-bit x86 `.app` binaries still exist.
pub(crate) const CPU_TYPE_I386: u32 = 7;
/// `CPU_TYPE_ARM`.
pub(crate) const CPU_TYPE_ARM: u32 = 12;
/// `CPU_TYPE_POWERPC`.
pub(crate) const CPU_TYPE_POWERPC: u32 = 18;
/// `CPU_TYPE_X86_64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_X86`.
pub(crate) const CPU_TYPE_X86_64: u32 = 0x0100_0007;
/// `CPU_TYPE_ARM64`, which is `CPU_ARCH_ABI64 | CPU_TYPE_ARM`.
pub(crate) const CPU_TYPE_ARM64: u32 = 0x0100_000c;
/// A CPU type no [`crate::Architecture`] variant names.
pub(crate) const CPU_TYPE_UNKNOWN: u32 = 0x0200_000c;

/// `LC_LOAD_DYLIB`: a library the binary needs at launch.
pub(crate) const LC_LOAD_DYLIB: u32 = 0x0c;
/// `LC_LOAD_WEAK_DYLIB`, which sets `LC_REQ_DYLD` (0x8000_0000).
pub(crate) const LC_LOAD_WEAK_DYLIB: u32 = 0x8000_0018;
/// `LC_REEXPORT_DYLIB`.
pub(crate) const LC_REEXPORT_DYLIB: u32 = 0x8000_001f;
/// `LC_LOAD_UPWARD_DYLIB`.
pub(crate) const LC_LOAD_UPWARD_DYLIB: u32 = 0x8000_0023;
/// `LC_VERSION_MIN_MACOSX`.
pub(crate) const LC_VERSION_MIN_MACOSX: u32 = 0x24;
/// `LC_BUILD_VERSION`.
pub(crate) const LC_BUILD_VERSION: u32 = 0x32;
/// `PLATFORM_MACOS`.
pub(crate) const PLATFORM_MACOS: u32 = 1;
/// `PLATFORM_IOS`, a platform a macOS binary must not declare.
pub(crate) const PLATFORM_IOS: u32 = 2;

/// Size of `dylib_command`, whose last fields are followed by the library path.
const DYLIB_COMMAND_LENGTH: u32 = 24;
/// Size of `build_version_command`.
const BUILD_VERSION_COMMAND_LENGTH: u32 = 24;
/// Size of `version_min_command`.
const VERSION_MIN_COMMAND_LENGTH: u32 = 16;
/// Offset of `ncmds` in both `mach_header` and `mach_header_64`, which share
/// their first `HEADER_32` bytes.
const NCMDS_OFFSET: usize = 16;
/// Offset of `sizeofcmds`, next to `ncmds`.
const SIZEOFCMDS_OFFSET: usize = 20;

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
pub(crate) fn write_xml_plist(bundle: &Path, xml: &str) -> PathBuf {
    let path = bundle.join("Contents").join("Info.plist");
    write_file(&path, xml.as_bytes());
    path
}

/// Writes the bundle's `Contents/Info.plist` as a binary property list made of
/// the given string entries, in the given order.
pub(crate) fn write_binary_plist(bundle: &Path, entries: &[(&str, &str)]) -> PathBuf {
    let mut dictionary = Dictionary::new();
    for (key, value) in entries {
        dictionary.insert((*key).to_string(), Value::String((*value).to_string()));
    }
    let path = bundle.join("Contents").join("Info.plist");
    create_dir(path.parent().expect("Info.plist lives in a directory"));
    Value::Dictionary(dictionary)
        .to_file_binary(&path)
        .expect("write the binary property list");
    path
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

/// A thin 32-bit little-endian Mach-O header for `cputype`, with no load
/// commands.
pub(crate) fn thin_macho_32(cputype: u32) -> Vec<u8> {
    let mut bytes = vec![0xce, 0xfa, 0xed, 0xfe];
    bytes.extend_from_slice(&cputype.to_le_bytes());
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&2u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes
}

/// A thin 32-bit big-endian Mach-O header for `cputype`, with no load commands.
pub(crate) fn thin_macho_big_endian(cputype: u32) -> Vec<u8> {
    let mut bytes = vec![0xfe, 0xed, 0xfa, 0xce];
    bytes.extend_from_slice(&cputype.to_be_bytes());
    bytes.extend_from_slice(&3u32.to_be_bytes());
    bytes.extend_from_slice(&2u32.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&0u32.to_be_bytes());
    bytes
}

/// A universal ("fat") Mach-O holding one 64-bit slice per entry of
/// `cputypes`, in that order, behind a big-endian 32-bit `fat_arch` table, which
/// is what `lipo` writes by default.
pub(crate) fn fat_macho(cputypes: &[u32]) -> Vec<u8> {
    universal(cputypes, false, false)
}

/// The same universal binary with the architecture table written little-endian
/// (`FAT_CIGAM`), which some packing tools produce.
pub(crate) fn fat_macho_little_endian(cputypes: &[u32]) -> Vec<u8> {
    universal(cputypes, true, false)
}

/// The same universal binary with a 64-bit `fat_arch_64` table
/// (`FAT_MAGIC_64`), used by files whose slices sit beyond the 4 GiB offsets a
/// 32-bit table can express.
pub(crate) fn fat_macho_wide(cputypes: &[u32]) -> Vec<u8> {
    universal(cputypes, false, true)
}

/// Builds a universal Mach-O: magic, `nfat_arch`, one architecture entry per
/// slice and then the slices themselves.
fn universal(cputypes: &[u32], little_endian: bool, wide: bool) -> Vec<u8> {
    let slices: Vec<(u32, Vec<u8>)> = cputypes
        .iter()
        .map(|cputype| (*cputype, thin_macho(*cputype)))
        .collect();
    universal_slices(&slices, little_endian, wide)
}

/// Builds a universal Mach-O out of the given `(cputype, slice)` pairs, in that
/// order.
fn universal_slices(slices: &[(u32, Vec<u8>)], little_endian: bool, wide: bool) -> Vec<u8> {
    let entry_length = if wide { 32 } else { 20 };
    let table_length = 8 + entry_length * slices.len();
    let magic: [u8; 4] = match (little_endian, wide) {
        (false, false) => [0xca, 0xfe, 0xba, 0xbe],
        (false, true) => [0xca, 0xfe, 0xba, 0xbf],
        (true, false) => [0xbe, 0xba, 0xfe, 0xca],
        (true, true) => [0xbf, 0xba, 0xfe, 0xca],
    };
    let put_u32 = |bytes: &mut Vec<u8>, value: u32| {
        if little_endian {
            bytes.extend_from_slice(&value.to_le_bytes());
        } else {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    };
    let put_u64 = |bytes: &mut Vec<u8>, value: u64| {
        if little_endian {
            bytes.extend_from_slice(&value.to_le_bytes());
        } else {
            bytes.extend_from_slice(&value.to_be_bytes());
        }
    };

    let mut bytes = magic.to_vec();
    put_u32(&mut bytes, slices.len() as u32);
    let mut offset = table_length as u32;
    for (cputype, slice) in slices {
        put_u32(&mut bytes, *cputype);
        put_u32(&mut bytes, 3); // cpusubtype: CPU_SUBTYPE_ANY
        if wide {
            put_u64(&mut bytes, u64::from(offset));
            put_u64(&mut bytes, slice.len() as u64);
            put_u32(&mut bytes, 0); // align: 2^0
            put_u32(&mut bytes, 0); // reserved
        } else {
            put_u32(&mut bytes, offset);
            put_u32(&mut bytes, slice.len() as u32);
            put_u32(&mut bytes, 0); // align: 2^0
        }
        offset += slice.len() as u32;
    }
    for (_, slice) in slices {
        bytes.extend_from_slice(slice);
    }
    bytes
}

/// A thin 64-bit little-endian Mach-O header for `cputype` carrying `commands`
/// as its load commands, with `ncmds` and `sizeofcmds` computed from them.
pub(crate) fn thin_macho_with(cputype: u32, commands: &[Vec<u8>]) -> Vec<u8> {
    header_with(cputype, commands, true)
}

/// A thin 32-bit little-endian Mach-O header for `cputype` carrying `commands`.
pub(crate) fn thin_macho_32_with(cputype: u32, commands: &[Vec<u8>]) -> Vec<u8> {
    header_with(cputype, commands, false)
}

/// A universal Mach-O whose slices carry the given load commands: one
/// `(cputype, commands)` entry per slice, in that order.
pub(crate) fn fat_macho_with(entries: &[(u32, Vec<Vec<u8>>)]) -> Vec<u8> {
    let slices: Vec<(u32, Vec<u8>)> = entries
        .iter()
        .map(|(cputype, commands)| (*cputype, thin_macho_with(*cputype, commands)))
        .collect();
    universal_slices(&slices, false, false)
}

/// An empty header of the requested width followed by `commands`.
fn header_with(cputype: u32, commands: &[Vec<u8>], wide: bool) -> Vec<u8> {
    let mut bytes = if wide {
        thin_macho(cputype)
    } else {
        thin_macho_32(cputype)
    };
    let sizeofcmds: usize = commands.iter().map(Vec::len).sum();
    bytes[NCMDS_OFFSET..SIZEOFCMDS_OFFSET].copy_from_slice(&(commands.len() as u32).to_le_bytes());
    bytes[SIZEOFCMDS_OFFSET..SIZEOFCMDS_OFFSET + 4]
        .copy_from_slice(&(sizeofcmds as u32).to_le_bytes());
    for command in commands {
        bytes.extend_from_slice(command);
    }
    bytes
}

/// An `LC_LOAD_DYLIB` command naming `name`.
pub(crate) fn dylib_command(name: &str) -> Vec<u8> {
    dylib_command_of(LC_LOAD_DYLIB, name)
}

/// The same command under another `cmd`, for the weak, reexport and upward
/// variants.
pub(crate) fn dylib_command_of(command: u32, name: &str) -> Vec<u8> {
    let mut text = name.as_bytes().to_vec();
    text.push(0);
    let unpadded = DYLIB_COMMAND_LENGTH as usize + text.len();
    let cmdsize = unpadded.div_ceil(4) * 4;
    let mut bytes = command.to_le_bytes().to_vec();
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

/// An `LC_VERSION_MIN_MACOSX` command declaring `version`.
pub(crate) fn version_min_macosx_command(version: u32) -> Vec<u8> {
    let mut bytes = LC_VERSION_MIN_MACOSX.to_le_bytes().to_vec();
    bytes.extend_from_slice(&VERSION_MIN_COMMAND_LENGTH.to_le_bytes());
    bytes.extend_from_slice(&version.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes()); // sdk
    bytes
}

/// Packs a version the way Mach-O does: 16 bits of major, 8 of minor, 8 of
/// patch.
pub(crate) fn packed_version(major: u32, minor: u32, patch: u32) -> u32 {
    (major << 16) | (minor << 8) | patch
}
