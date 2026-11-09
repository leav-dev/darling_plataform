//! Fixtures shared by the crate's tests.
//!
//! Every fixture is a property list written from a literal and a Mach-O header
//! built from explicit bytes inside a temporary directory. The tests never
//! depend on a real `.app` bundle being installed on the host.

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
    let slices: Vec<Vec<u8>> = cputypes
        .iter()
        .map(|cputype| thin_macho(*cputype))
        .collect();
    let entry_length = if wide { 32 } else { 20 };
    let table_length = 8 + entry_length * cputypes.len();
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
    put_u32(&mut bytes, cputypes.len() as u32);
    let mut offset = table_length as u32;
    for (cputype, slice) in cputypes.iter().zip(&slices) {
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
    for slice in slices {
        bytes.extend_from_slice(&slice);
    }
    bytes
}
