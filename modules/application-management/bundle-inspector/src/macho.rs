//! Mach-O header parsing.
//!
//! The executable of a bundle is untrusted input, so only the fixed header and,
//! for a universal binary, the architecture table are read: the parser needs to
//! know which slices exist, never what they contain. Nothing is executed and no
//! external tool (`file`, `otool`, `lipo`) is consulted.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use serde::Serialize;

use crate::{Error, Result};

/// The CPU architecture of one Mach-O slice.
///
/// Thin binaries report a single architecture; a universal binary reports one
/// entry per slice, in the order the file declares them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Architecture {
    /// `CPU_TYPE_X86_64`: 64-bit Intel.
    X86_64,
    /// `CPU_TYPE_ARM64`: Apple silicon.
    Arm64,
    /// `CPU_TYPE_ARM`: 32-bit ARM.
    Arm,
    /// `CPU_TYPE_I386`: 32-bit Intel.
    I386,
    /// `CPU_TYPE_POWERPC`: PowerPC. Spelled `powerpc`, as Apple's tooling does.
    #[serde(rename = "powerpc")]
    PowerPc,
    /// A CPU type this build does not name, reported verbatim so the caller can
    /// still decide compatibility.
    Unknown(u32),
}

impl Architecture {
    /// Maps a Mach-O `cputype` to the reported architecture.
    fn from_cpu_type(cputype: u32) -> Self {
        match cputype {
            CPU_TYPE_X86_64 => Self::X86_64,
            CPU_TYPE_ARM64 => Self::Arm64,
            CPU_TYPE_ARM => Self::Arm,
            CPU_TYPE_I386 => Self::I386,
            CPU_TYPE_POWERPC => Self::PowerPc,
            other => Self::Unknown(other),
        }
    }
}

// `cputype` values from `<mach/machine.h>`. The 64-bit architectures set
// `CPU_ARCH_ABI64` (0x0100_0000) in the high byte.
const CPU_TYPE_I386: u32 = 7;
const CPU_TYPE_ARM: u32 = 12;
const CPU_TYPE_POWERPC: u32 = 18;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;

// Magic numbers as they appear on disk, one array per byte order and width.
// `MH_MAGIC` stored little-endian is `ce fa ed fe`; stored big-endian it is
// `fe ed fa ce`, which is the same constant read the other way round.
const MAGIC_32_LITTLE_ENDIAN: [u8; 4] = [0xce, 0xfa, 0xed, 0xfe];
const MAGIC_32_BIG_ENDIAN: [u8; 4] = [0xfe, 0xed, 0xfa, 0xce];
const MAGIC_64_LITTLE_ENDIAN: [u8; 4] = [0xcf, 0xfa, 0xed, 0xfe];
const MAGIC_64_BIG_ENDIAN: [u8; 4] = [0xfe, 0xed, 0xfa, 0xcf];
const MAGIC_FAT_BIG_ENDIAN_32: [u8; 4] = [0xca, 0xfe, 0xba, 0xbe];
const MAGIC_FAT_BIG_ENDIAN_64: [u8; 4] = [0xca, 0xfe, 0xba, 0xbf];
const MAGIC_FAT_LITTLE_ENDIAN_32: [u8; 4] = [0xbe, 0xba, 0xfe, 0xca];
const MAGIC_FAT_LITTLE_ENDIAN_64: [u8; 4] = [0xbf, 0xba, 0xfe, 0xca];

/// Size of `mach_header`.
const HEADER_32_LENGTH: u64 = 28;
/// Size of `mach_header_64`.
const HEADER_64_LENGTH: u64 = 32;
/// Magic plus `nfat_arch`.
const FAT_HEADER_LENGTH: u64 = 8;
/// Size of one `fat_arch` entry.
const FAT_ARCH_LENGTH_32: u64 = 20;
/// Size of one `fat_arch_64` entry.
const FAT_ARCH_LENGTH_64: u64 = 32;

/// Byte order of the header fields that follow the magic.
#[derive(Clone, Copy)]
enum ByteOrder {
    Little,
    Big,
}

impl ByteOrder {
    fn u32(self, bytes: &[u8]) -> u32 {
        let bytes: [u8; 4] = bytes.try_into().expect("a four byte field");
        match self {
            Self::Little => u32::from_le_bytes(bytes),
            Self::Big => u32::from_be_bytes(bytes),
        }
    }

    fn u64(self, bytes: &[u8]) -> u64 {
        let bytes: [u8; 8] = bytes.try_into().expect("an eight byte field");
        match self {
            Self::Little => u64::from_le_bytes(bytes),
            Self::Big => u64::from_be_bytes(bytes),
        }
    }
}

/// Reads the architectures of the Mach-O file at `executable`.
///
/// Only the header is read: a thin header of either width and byte order, or the
/// `fat_arch` table of a universal binary. Slices are reported in file order and
/// de-duplicated, so a file that repeats a slice reports it once.
pub fn architectures(executable: &Path) -> Result<Vec<Architecture>> {
    let mut file = File::open(executable).map_err(|source| io_error(executable, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(executable, source))?
        .len();
    let mut head = Vec::with_capacity(FAT_HEADER_LENGTH as usize);
    file.by_ref()
        .take(FAT_HEADER_LENGTH)
        .read_to_end(&mut head)
        .map_err(|source| io_error(executable, source))?;
    let head_length = head.len() as u64;
    if head_length < 4 {
        return Err(truncated(executable, 4, head_length));
    }
    let magic: [u8; 4] = head[..4].try_into().expect("four magic bytes");

    let (order, header_length) = match magic {
        MAGIC_32_LITTLE_ENDIAN => (ByteOrder::Little, HEADER_32_LENGTH),
        MAGIC_32_BIG_ENDIAN => (ByteOrder::Big, HEADER_32_LENGTH),
        MAGIC_64_LITTLE_ENDIAN => (ByteOrder::Little, HEADER_64_LENGTH),
        MAGIC_64_BIG_ENDIAN => (ByteOrder::Big, HEADER_64_LENGTH),
        MAGIC_FAT_BIG_ENDIAN_32 => {
            return fat_architectures(
                &mut file,
                executable,
                length,
                head_length,
                ByteOrder::Big,
                FAT_ARCH_LENGTH_32,
            );
        }
        MAGIC_FAT_BIG_ENDIAN_64 => {
            return fat_architectures(
                &mut file,
                executable,
                length,
                head_length,
                ByteOrder::Big,
                FAT_ARCH_LENGTH_64,
            );
        }
        MAGIC_FAT_LITTLE_ENDIAN_32 => {
            return fat_architectures(
                &mut file,
                executable,
                length,
                head_length,
                ByteOrder::Little,
                FAT_ARCH_LENGTH_32,
            );
        }
        MAGIC_FAT_LITTLE_ENDIAN_64 => {
            return fat_architectures(
                &mut file,
                executable,
                length,
                head_length,
                ByteOrder::Little,
                FAT_ARCH_LENGTH_64,
            );
        }
        _ => {
            return Err(Error::NotMachO {
                path: executable.to_path_buf(),
                magic,
            });
        }
    };

    if length < header_length {
        return Err(truncated(executable, header_length, length));
    }
    // A header that fits is at least eight bytes long, because both widths are,
    // so the `cputype` field is already in `head`.
    let cputype = order.u32(&head[4..8]);
    Ok(vec![Architecture::from_cpu_type(cputype)])
}

/// Reads the architecture table of a universal binary and collects one
/// architecture per slice that the file is actually long enough to hold.
fn fat_architectures(
    file: &mut File,
    executable: &Path,
    length: u64,
    head_length: u64,
    order: ByteOrder,
    entry_length: u64,
) -> Result<Vec<Architecture>> {
    if head_length < FAT_HEADER_LENGTH {
        return Err(truncated(executable, FAT_HEADER_LENGTH, head_length));
    }
    // The magic and `nfat_arch` are already in the first eight bytes of `file`.
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(executable, source))?;
    let mut prefix = [0u8; FAT_HEADER_LENGTH as usize];
    file.read_exact(&mut prefix)
        .map_err(|source| io_error(executable, source))?;
    let slices = u64::from(order.u32(&prefix[4..8]));
    if slices == 0 {
        return Err(Error::MachOInvalidFat {
            path: executable.to_path_buf(),
            reason: String::from("it declares no slices"),
        });
    }
    let table_length = FAT_HEADER_LENGTH + slices * entry_length;
    if length < table_length {
        return Err(truncated(executable, table_length, length));
    }

    let table_length = usize::try_from(table_length).map_err(|_| Error::MachOInvalidFat {
        path: executable.to_path_buf(),
        reason: String::from("its architecture table is larger than this platform can address"),
    })?;
    let mut table = vec![0u8; table_length];
    file.seek(SeekFrom::Start(0))
        .map_err(|source| io_error(executable, source))?;
    file.read_exact(&mut table)
        .map_err(|source| io_error(executable, source))?;

    let entry_length = entry_length as usize;
    let mut architectures = Vec::new();
    for (index, entry) in table[FAT_HEADER_LENGTH as usize..]
        .chunks_exact(entry_length)
        .enumerate()
    {
        let cputype = order.u32(&entry[0..4]);
        let (offset, size) = if entry_length == FAT_ARCH_LENGTH_32 as usize {
            (
                u64::from(order.u32(&entry[8..12])),
                u64::from(order.u32(&entry[12..16])),
            )
        } else {
            (order.u64(&entry[8..16]), order.u64(&entry[16..24]))
        };
        let end = offset
            .checked_add(size)
            .ok_or_else(|| Error::MachOInvalidFat {
                path: executable.to_path_buf(),
                reason: format!("slice {index} at offset {offset} overflows the file"),
            })?;
        if end > length {
            return Err(Error::MachOInvalidFat {
                path: executable.to_path_buf(),
                reason: format!(
                    "slice {index} spans {size} bytes at offset {offset}, past the file length \
                     {length}"
                ),
            });
        }
        let architecture = Architecture::from_cpu_type(cputype);
        if !architectures.contains(&architecture) {
            architectures.push(architecture);
        }
    }
    Ok(architectures)
}

fn truncated(path: &Path, needed: u64, found: u64) -> Error {
    Error::MachOTruncated {
        path: path.to_path_buf(),
        needed,
        found,
    }
}

fn io_error(path: &Path, source: std::io::Error) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{
        CPU_TYPE_ARM, CPU_TYPE_ARM64, CPU_TYPE_I386, CPU_TYPE_POWERPC, CPU_TYPE_UNKNOWN,
        CPU_TYPE_X86_64, fat_macho, fat_macho_little_endian, fat_macho_wide, thin_macho,
        thin_macho_32, thin_macho_big_endian, write_file,
    };

    /// Writes `bytes` to a temporary file and returns the directory that owns it
    /// together with its path.
    fn executable(bytes: &[u8]) -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let path = directory.path().join("Executable");
        write_file(&path, bytes);
        (directory, path)
    }

    #[test]
    fn reads_a_thin_64_bit_little_endian_header() {
        let (_directory, path) = executable(&thin_macho(CPU_TYPE_X86_64));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::X86_64]
        );
    }

    #[test]
    fn reads_a_thin_arm64_header() {
        let (_directory, path) = executable(&thin_macho(CPU_TYPE_ARM64));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::Arm64]
        );
    }

    #[test]
    fn reads_a_thin_32_bit_little_endian_header() {
        let (_directory, path) = executable(&thin_macho_32(CPU_TYPE_I386));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::I386]
        );
    }

    #[test]
    fn reads_a_thin_32_bit_big_endian_header() {
        let (_directory, path) = executable(&thin_macho_big_endian(CPU_TYPE_POWERPC));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::PowerPc]
        );
    }

    #[test]
    fn reads_the_slices_of_a_universal_header_in_file_order() {
        let (_directory, path) = executable(&fat_macho(&[CPU_TYPE_ARM64, CPU_TYPE_X86_64]));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::Arm64, Architecture::X86_64]
        );
    }

    #[test]
    fn reads_a_little_endian_universal_header() {
        let (_directory, path) = executable(&fat_macho_little_endian(&[CPU_TYPE_ARM64]));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::Arm64]
        );
    }

    #[test]
    fn reads_a_wide_universal_header() {
        let (_directory, path) = executable(&fat_macho_wide(&[CPU_TYPE_X86_64, CPU_TYPE_ARM64]));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::X86_64, Architecture::Arm64]
        );
    }

    #[test]
    fn de_duplicates_repeated_slices() {
        let (_directory, path) = executable(&fat_macho(&[
            CPU_TYPE_X86_64,
            CPU_TYPE_ARM64,
            CPU_TYPE_X86_64,
        ]));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::X86_64, Architecture::Arm64]
        );
    }

    #[test]
    fn reports_an_unknown_cpu_type_verbatim() {
        let (_directory, path) = executable(&thin_macho(CPU_TYPE_UNKNOWN));

        assert_eq!(
            architectures(&path).expect("parse"),
            vec![Architecture::Unknown(CPU_TYPE_UNKNOWN)]
        );
    }

    #[test]
    fn maps_every_named_cpu_type() {
        let arm = executable(&thin_macho(CPU_TYPE_ARM));
        assert_eq!(
            architectures(&arm.1).expect("parse"),
            vec![Architecture::Arm]
        );
    }

    #[test]
    fn rejects_a_file_shorter_than_the_magic() {
        let (_directory, path) = executable(&[0xcf, 0xfa]);

        let error = architectures(&path).expect_err("two bytes are not a header");

        assert!(
            matches!(
                error,
                Error::MachOTruncated {
                    needed: 4,
                    found: 2,
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_a_thin_header_cut_short() {
        let header = thin_macho(CPU_TYPE_X86_64);
        let (_directory, path) = executable(&header[..header.len() - 1]);

        let error = architectures(&path).expect_err("the header is incomplete");

        assert!(
            matches!(
                error,
                Error::MachOTruncated {
                    needed: 32,
                    found: 31,
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_a_universal_header_without_slices() {
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe];
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let (_directory, path) = executable(&bytes);

        let error = architectures(&path).expect_err("a universal binary has at least one slice");

        assert!(
            matches!(error, Error::MachOInvalidFat { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("no slices"),
            "the message must say what is wrong: {error}"
        );
    }

    #[test]
    fn rejects_a_universal_header_whose_table_does_not_fit() {
        let mut bytes = vec![0xca, 0xfe, 0xba, 0xbe];
        bytes.extend_from_slice(&3u32.to_be_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        let (_directory, path) = executable(&bytes);

        let error = architectures(&path).expect_err("the table is truncated");

        assert!(
            matches!(
                error,
                Error::MachOTruncated {
                    needed: 68,
                    found: 16,
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_a_slice_that_points_outside_the_file() {
        let mut bytes = fat_macho(&[CPU_TYPE_X86_64]);
        // The single `fat_arch` entry starts at byte 8; its size is the fourth
        // field, so it ends at byte 20.
        bytes[16..20].copy_from_slice(&4096u32.to_be_bytes());
        let (_directory, path) = executable(&bytes);

        let error = architectures(&path).expect_err("a slice outside the file is corrupt");

        assert!(
            matches!(error, Error::MachOInvalidFat { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("4096"),
            "the message must report the offending size: {error}"
        );
    }

    #[test]
    fn a_file_that_is_not_mach_o_names_the_path_and_the_magic() {
        let (_directory, path) = executable(b"\x7fELF\x02\x01\x01\x00");

        let error = architectures(&path).expect_err("ELF is not Mach-O");
        let message = error.to_string();

        assert!(
            matches!(error, Error::NotMachO { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains(&path.display().to_string()),
            "the message must name the path: {message}"
        );
        assert!(
            message.contains("7f 45 4c 46"),
            "the message must report the observed magic: {message}"
        );
    }

    #[test]
    fn a_missing_executable_is_an_io_error() {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let path = directory.path().join("Absent");

        let error = architectures(&path).expect_err("the file does not exist");

        assert!(
            matches!(error, Error::Io { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "the message must name the path: {error}"
        );
    }
}
