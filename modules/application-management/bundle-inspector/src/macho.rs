//! Mach-O header and load command parsing.
//!
//! The executable of a bundle is untrusted input, so only the fixed headers, the
//! architecture table of a universal binary and the load commands are read: the
//! parser needs to know which slices exist and what they ask for, never what
//! they contain. Nothing is executed and no external tool (`file`, `otool`,
//! `lipo`, `dyld`) is consulted.

use std::collections::BTreeSet;
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
/// Size of a Mach-O magic.
const MAGIC_LENGTH: u64 = 4;
/// Offset of `sizeofcmds` in both `mach_header` and `mach_header_64`, which
/// share their first 28 bytes.
const SIZEOFCMDS_OFFSET: usize = 20;
/// Size of `load_command`, the header every load command starts with.
const LOAD_COMMAND_LENGTH: u64 = 8;
/// Size of `dylib_command`, which carries the library path after its fixed
/// fields.
const DYLIB_COMMAND_LENGTH: u64 = 24;
/// Size of `build_version_command`.
const BUILD_VERSION_COMMAND_LENGTH: u64 = 24;
/// Size of `version_min_command`.
const VERSION_MIN_COMMAND_LENGTH: u64 = 16;
/// Magic plus `nfat_arch`.
const FAT_HEADER_LENGTH: u64 = 8;
/// Size of one `fat_arch` entry.
const FAT_ARCH_LENGTH_32: u64 = 20;
/// Size of one `fat_arch_64` entry.
const FAT_ARCH_LENGTH_64: u64 = 32;

// Load command types from `<mach-o/loader.h>`.

/// `LC_REQ_DYLD`, which the optional `dyld` command types set in their high bit.
const LC_REQ_DYLD: u32 = 0x8000_0000;
/// `LC_LOAD_DYLIB`: a library the binary needs at launch.
const LC_LOAD_DYLIB: u32 = 0x0c;
/// `LC_LOAD_WEAK_DYLIB`: a library the binary needs when it is present.
const LC_LOAD_WEAK_DYLIB: u32 = LC_REQ_DYLD | 0x18;
/// `LC_REEXPORT_DYLIB`: a library whose symbols this binary re-exports.
const LC_REEXPORT_DYLIB: u32 = LC_REQ_DYLD | 0x1f;
/// `LC_LOAD_UPWARD_DYLIB`: a library this binary may satisfy the symbols of.
const LC_LOAD_UPWARD_DYLIB: u32 = LC_REQ_DYLD | 0x23;
/// `LC_VERSION_MIN_MACOSX`: the legacy minimum macOS declaration.
const LC_VERSION_MIN_MACOSX: u32 = 0x24;
/// `LC_BUILD_VERSION`: the platform and minimum OS declaration that replaced
/// `LC_VERSION_MIN_MACOSX`.
const LC_BUILD_VERSION: u32 = 0x32;
/// `PLATFORM_MACOS`, the only `LC_BUILD_VERSION` platform that describes a macOS
/// requirement; every other value names a different operating system.
const PLATFORM_MACOS: u32 = 1;
/// Darwin's `MAXPATHLEN`. A longer library path cannot be loaded, so a name that
/// carries no terminating zero inside that many bytes is malformed.
const MAX_DYLIB_PATH_LENGTH: u64 = 1024;

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

/// One Mach-O image inside the file: the CPU type from the header or from the
/// architecture table, and the byte range the slice occupies.
struct Image {
    cputype: u32,
    offset: u64,
    length: u64,
}

/// Reads the architectures of the Mach-O file at `executable`.
///
/// Only the headers are read: a thin header of either width and byte order, or
/// the `fat_arch` table of a universal binary. Slices are reported in file order
/// and de-duplicated, so a file that repeats a slice reports it once.
pub fn architectures(executable: &Path) -> Result<Vec<Architecture>> {
    let mut file = File::open(executable).map_err(|source| io_error(executable, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(executable, source))?
        .len();
    let mut architectures = Vec::new();
    for image in images(&mut file, executable, length)? {
        record_architecture(&mut architectures, image.cputype);
    }
    Ok(architectures)
}

/// Records the architecture of `cputype` unless it is already reported, so a
/// file that repeats a slice reports the architecture once.
fn record_architecture(architectures: &mut Vec<Architecture>, cputype: u32) {
    let architecture = Architecture::from_cpu_type(cputype);
    if !architectures.contains(&architecture) {
        architectures.push(architecture);
    }
}

/// Reads the images the file holds: one for a thin binary, or one per entry of a
/// universal binary's architecture table. Every image is proven to sit inside
/// the file.
fn images(file: &mut File, executable: &Path, length: u64) -> Result<Vec<Image>> {
    let mut head = Vec::with_capacity(FAT_HEADER_LENGTH as usize);
    file.by_ref()
        .take(FAT_HEADER_LENGTH)
        .read_to_end(&mut head)
        .map_err(|source| io_error(executable, source))?;
    let head_length = head.len() as u64;
    if head_length < MAGIC_LENGTH {
        return Err(truncated(executable, MAGIC_LENGTH, head_length));
    }
    let magic: [u8; MAGIC_LENGTH as usize] = head[..MAGIC_LENGTH as usize]
        .try_into()
        .expect("four magic bytes");

    let (order, header_length) = match magic {
        MAGIC_32_LITTLE_ENDIAN => (ByteOrder::Little, HEADER_32_LENGTH),
        MAGIC_32_BIG_ENDIAN => (ByteOrder::Big, HEADER_32_LENGTH),
        MAGIC_64_LITTLE_ENDIAN => (ByteOrder::Little, HEADER_64_LENGTH),
        MAGIC_64_BIG_ENDIAN => (ByteOrder::Big, HEADER_64_LENGTH),
        MAGIC_FAT_BIG_ENDIAN_32 => {
            return fat_images(
                file,
                executable,
                length,
                head_length,
                ByteOrder::Big,
                FAT_ARCH_LENGTH_32,
            );
        }
        MAGIC_FAT_BIG_ENDIAN_64 => {
            return fat_images(
                file,
                executable,
                length,
                head_length,
                ByteOrder::Big,
                FAT_ARCH_LENGTH_64,
            );
        }
        MAGIC_FAT_LITTLE_ENDIAN_32 => {
            return fat_images(
                file,
                executable,
                length,
                head_length,
                ByteOrder::Little,
                FAT_ARCH_LENGTH_32,
            );
        }
        MAGIC_FAT_LITTLE_ENDIAN_64 => {
            return fat_images(
                file,
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
    Ok(vec![Image {
        cputype,
        offset: 0,
        length,
    }])
}

/// Reads the architecture table of a universal binary and collects one image per
/// slice that the file is actually long enough to hold.
fn fat_images(
    file: &mut File,
    executable: &Path,
    length: u64,
    head_length: u64,
    order: ByteOrder,
    entry_length: u64,
) -> Result<Vec<Image>> {
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
    let mut images = Vec::new();
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
        images.push(Image {
            cputype,
            offset,
            length: size,
        });
    }
    Ok(images)
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

/// Everything the load commands of one Mach-O executable declare about what it
/// needs in order to run.
///
/// These are the binary's own declarations, never a judgement: whether this host
/// can provide them is decided elsewhere. A favourable report says what the
/// binary asks for, never that it will run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BinaryRequirements {
    /// Architectures of the executable, in slice order and de-duplicated.
    pub architectures: Vec<Architecture>,
    /// Library paths from the dylib load commands, verbatim as the binary
    /// declares them, sorted and de-duplicated across slices. They are install
    /// names, so they may be absolute (`/usr/lib/libz.1.dylib`) or relative to
    /// the loader (`@rpath/Foo.framework/Versions/A/Foo`); nothing resolves
    /// them. An empty name names nothing and is not reported.
    pub dylibs: Vec<String>,
    /// The declared minimum macOS version as `major.minor.patch`, or `None` when
    /// no load command declares one. `LC_BUILD_VERSION.minos` takes precedence
    /// over `LC_VERSION_MIN_MACOSX.version`, and a universal binary reports the
    /// highest minimum any slice declares, because the caller cannot know which
    /// slice will run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum_os: Option<String>,
}

/// What the load commands of one slice declare.
struct DeclaredCommands {
    dylibs: Vec<String>,
    build_version: Option<u32>,
    version_min: Option<u32>,
}

/// Reads the requirements the load commands of the Mach-O file at `executable`
/// declare, without executing it and without resolving anything it names.
///
/// Every slice is walked, so a universal binary reports the union of its dylibs
/// and the architectures of all its slices. Only the four dylib commands and the
/// two version commands are interpreted; every other command is skipped by its
/// declared size, so no payload of an untrusted file is parsed. A malformed
/// header, command table or library path is rejected instead of reported.
pub fn requirements(executable: &Path) -> Result<BinaryRequirements> {
    let mut file = File::open(executable).map_err(|source| io_error(executable, source))?;
    let length = file
        .metadata()
        .map_err(|source| io_error(executable, source))?
        .len();
    let images = images(&mut file, executable, length)?;

    let mut architectures = Vec::new();
    let mut dylibs = BTreeSet::new();
    let mut build_version = None;
    let mut version_min = None;
    for image in &images {
        record_architecture(&mut architectures, image.cputype);
        let declared = declared_commands(&mut file, executable, image)?;
        dylibs.extend(declared.dylibs);
        build_version = highest(build_version, declared.build_version);
        version_min = highest(version_min, declared.version_min);
    }

    Ok(BinaryRequirements {
        architectures,
        dylibs: dylibs.into_iter().collect(),
        minimum_os: build_version.or(version_min).map(render_version),
    })
}

/// Reads the load commands of one image.
///
/// The walk is bounded by `sizeofcmds`, the byte extent the header declares,
/// rather than by `ncmds`: a count cannot make the parser skip a command or read
/// past the extent it was given.
fn declared_commands(
    file: &mut File,
    executable: &Path,
    image: &Image,
) -> Result<DeclaredCommands> {
    let (order, header_length) = slice_header(file, executable, image)?;
    if image.length < header_length {
        return Err(truncated(executable, header_length, image.length));
    }
    let field: [u8; 4] = fields(file, executable, image.offset + SIZEOFCMDS_OFFSET as u64)?;
    let sizeofcmds = u64::from(order.u32(&field));
    if header_length + sizeofcmds > image.length {
        return Err(truncated(
            executable,
            header_length + sizeofcmds,
            image.length,
        ));
    }

    let mut declared = DeclaredCommands {
        dylibs: Vec::new(),
        build_version: None,
        version_min: None,
    };
    let mut position = image.offset + header_length;
    let mut remaining = sizeofcmds;
    while remaining > 0 {
        if remaining < LOAD_COMMAND_LENGTH {
            return Err(invalid_load_command(
                executable,
                format!(
                    "the load commands end with {remaining} bytes at offset {position}, which \
                     cannot hold the {LOAD_COMMAND_LENGTH}-byte load_command they announce"
                ),
            ));
        }
        let head: [u8; LOAD_COMMAND_LENGTH as usize] = fields(file, executable, position)?;
        let command = order.u32(&head[0..4]);
        let size = u64::from(order.u32(&head[4..8]));
        if size < LOAD_COMMAND_LENGTH {
            return Err(invalid_load_command(
                executable,
                format!(
                    "the command at offset {position} declares cmdsize {size}, which cannot hold \
                     the {LOAD_COMMAND_LENGTH}-byte load_command header"
                ),
            ));
        }
        if size > remaining {
            return Err(invalid_load_command(
                executable,
                format!(
                    "the command at offset {position} declares cmdsize {size}, past the \
                     {sizeofcmds} bytes its header declares"
                ),
            ));
        }
        match command {
            LC_LOAD_DYLIB | LC_LOAD_WEAK_DYLIB | LC_REEXPORT_DYLIB | LC_LOAD_UPWARD_DYLIB => {
                if size < DYLIB_COMMAND_LENGTH {
                    return Err(too_small(
                        executable,
                        position,
                        size,
                        DYLIB_COMMAND_LENGTH,
                        "dylib_command",
                    ));
                }
                let fixed: [u8; DYLIB_COMMAND_LENGTH as usize] =
                    fields(file, executable, position)?;
                let name_offset = u64::from(order.u32(&fixed[8..12]));
                let name = dylib_name(file, executable, position, size, name_offset)?;
                if !name.is_empty() {
                    declared.dylibs.push(name);
                }
            }
            LC_BUILD_VERSION => {
                if size < BUILD_VERSION_COMMAND_LENGTH {
                    return Err(too_small(
                        executable,
                        position,
                        size,
                        BUILD_VERSION_COMMAND_LENGTH,
                        "build_version_command",
                    ));
                }
                let fixed: [u8; BUILD_VERSION_COMMAND_LENGTH as usize] =
                    fields(file, executable, position)?;
                let platform = order.u32(&fixed[8..12]);
                let minos = order.u32(&fixed[12..16]);
                // A command for another platform says nothing about macOS, so it
                // is skipped instead of being misreported as a macOS minimum.
                if platform == PLATFORM_MACOS {
                    declared.build_version = highest(declared.build_version, Some(minos));
                }
            }
            LC_VERSION_MIN_MACOSX => {
                if size < VERSION_MIN_COMMAND_LENGTH {
                    return Err(too_small(
                        executable,
                        position,
                        size,
                        VERSION_MIN_COMMAND_LENGTH,
                        "version_min_command",
                    ));
                }
                let fixed: [u8; VERSION_MIN_COMMAND_LENGTH as usize] =
                    fields(file, executable, position)?;
                let version = order.u32(&fixed[8..12]);
                declared.version_min = highest(declared.version_min, Some(version));
            }
            _ => {}
        }
        position += size;
        remaining -= size;
    }
    Ok(declared)
}

/// Reads the magic of the image at `image.offset` and reports the byte order and
/// the header width it declares.
///
/// A thin binary's magic is the file's own and was already matched by [`images`];
/// every slice of a universal binary declares its own, so a slice that does not
/// start with a Mach-O header leaves the universal binary corrupt.
fn slice_header(file: &mut File, executable: &Path, image: &Image) -> Result<(ByteOrder, u64)> {
    if image.length < MAGIC_LENGTH {
        return Err(truncated(executable, MAGIC_LENGTH, image.length));
    }
    let magic: [u8; MAGIC_LENGTH as usize] = fields(file, executable, image.offset)?;
    match magic {
        MAGIC_32_LITTLE_ENDIAN => Ok((ByteOrder::Little, HEADER_32_LENGTH)),
        MAGIC_32_BIG_ENDIAN => Ok((ByteOrder::Big, HEADER_32_LENGTH)),
        MAGIC_64_LITTLE_ENDIAN => Ok((ByteOrder::Little, HEADER_64_LENGTH)),
        MAGIC_64_BIG_ENDIAN => Ok((ByteOrder::Big, HEADER_64_LENGTH)),
        _ => Err(Error::MachOInvalidFat {
            path: executable.to_path_buf(),
            reason: format!(
                "the slice at offset {} does not start with a Mach-O header",
                image.offset
            ),
        }),
    }
}

/// Reads the library path of one dylib command, proving that the range it names
/// stays inside the command the header declared, and returns it verbatim so the
/// caller reports the install name the binary asks for.
fn dylib_name(
    file: &mut File,
    executable: &Path,
    position: u64,
    size: u64,
    name_offset: u64,
) -> Result<String> {
    if name_offset < DYLIB_COMMAND_LENGTH || name_offset >= size {
        return Err(invalid_load_command(
            executable,
            format!(
                "the dylib command at offset {position} names its library at offset \
                 {name_offset}, outside the {size} bytes of the command it has to fit in after \
                 its {DYLIB_COMMAND_LENGTH}-byte structure"
            ),
        ));
    }
    // The command was already proven to end inside the slice, so this range is
    // inside the slice too. Darwin limits a path to `MAXPATHLEN`, so a name that
    // has not terminated by then is malformed rather than merely long.
    let available = (size - name_offset).min(MAX_DYLIB_PATH_LENGTH);
    let mut name = vec![0u8; available as usize];
    file.seek(SeekFrom::Start(position + name_offset))
        .map_err(|source| io_error(executable, source))?;
    file.read_exact(&mut name)
        .map_err(|source| io_error(executable, source))?;
    let end = name.iter().position(|byte| *byte == 0).ok_or_else(|| {
        invalid_load_command(
            executable,
            format!(
                "the library named at offset {name_offset} of the command at offset {position} \
                 carries no terminating zero inside the {available} bytes available to it"
            ),
        )
    })?;
    let text = std::str::from_utf8(&name[..end]).map_err(|_| {
        invalid_load_command(
            executable,
            format!(
                "the library named at offset {name_offset} of the command at offset {position} \
                 is not valid UTF-8"
            ),
        )
    })?;
    Ok(text.to_string())
}

/// Reads exactly the `N` bytes that start at `position`, so a command is parsed
/// from the bytes a fixed structure occupies and never from a speculative length.
fn fields<const N: usize>(file: &mut File, executable: &Path, position: u64) -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    file.seek(SeekFrom::Start(position))
        .map_err(|source| io_error(executable, source))?;
    file.read_exact(&mut bytes)
        .map_err(|source| io_error(executable, source))?;
    Ok(bytes)
}

/// The larger of two declared versions, so a universal binary reports the
/// strictest minimum its slices declare.
fn highest(current: Option<u32>, declared: Option<u32>) -> Option<u32> {
    match (current, declared) {
        (Some(current), Some(declared)) => Some(current.max(declared)),
        (current, declared) => current.or(declared),
    }
}

/// Renders a Mach-O version, which packs 16 bits of major, 8 of minor and 8 of
/// patch, as `major.minor.patch`. A version that declares no patch reports `0`,
/// so every rendering has the same shape.
fn render_version(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version >> 16,
        (version >> 8) & 0xff,
        version & 0xff
    )
}

fn invalid_load_command(path: &Path, reason: String) -> Error {
    Error::MachOInvalidLoadCommand {
        path: path.to_path_buf(),
        reason,
    }
}

/// Names the command that declares less room than its own structure needs.
fn too_small(path: &Path, position: u64, size: u64, needed: u64, structure: &str) -> Error {
    invalid_load_command(
        path,
        format!(
            "the command at offset {position} declares cmdsize {size}, which cannot hold the \
             {needed}-byte {structure} it announces"
        ),
    )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;
    use crate::test_support::{
        CPU_TYPE_ARM, CPU_TYPE_ARM64, CPU_TYPE_I386, CPU_TYPE_POWERPC, CPU_TYPE_UNKNOWN,
        CPU_TYPE_X86_64, LC_LOAD_UPWARD_DYLIB, LC_LOAD_WEAK_DYLIB, LC_REEXPORT_DYLIB, PLATFORM_IOS,
        PLATFORM_MACOS, build_version_command, dylib_command, dylib_command_of, fat_macho,
        fat_macho_little_endian, fat_macho_wide, fat_macho_with, packed_version, thin_macho,
        thin_macho_32, thin_macho_32_with, thin_macho_big_endian, thin_macho_with,
        version_min_macosx_command, write_file,
    };

    /// Writes `bytes` to a temporary file and returns the directory that owns it
    /// together with its path.
    fn executable(bytes: &[u8]) -> (TempDir, PathBuf) {
        let directory = tempfile::tempdir().expect("create the temporary directory");
        let path = directory.path().join("Executable");
        write_file(&path, bytes);
        (directory, path)
    }

    /// The requirements of a thin ARM64 binary carrying `commands`.
    fn thin_requirements(commands: &[Vec<u8>]) -> BinaryRequirements {
        let (_directory, path) = executable(&thin_macho_with(CPU_TYPE_ARM64, commands));
        requirements(&path).expect("the headers and load commands are well formed")
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

    #[test]
    fn reports_the_dylibs_of_a_thin_binary_sorted() {
        let requirements = thin_requirements(&[
            dylib_command("/usr/lib/libz.1.dylib"),
            dylib_command("@rpath/Foo.framework/Versions/A/Foo"),
        ]);

        assert_eq!(requirements.architectures, vec![Architecture::Arm64]);
        assert_eq!(
            requirements.dylibs,
            vec![
                String::from("/usr/lib/libz.1.dylib"),
                String::from("@rpath/Foo.framework/Versions/A/Foo"),
            ]
        );
        assert_eq!(requirements.minimum_os, None);
    }

    #[test]
    fn reports_every_dylib_command_kind() {
        let requirements = thin_requirements(&[
            dylib_command_of(LC_LOAD_WEAK_DYLIB, "@rpath/Weak.framework/Weak"),
            dylib_command_of(LC_REEXPORT_DYLIB, "@rpath/Reexport.framework/Reexport"),
            dylib_command_of(LC_LOAD_UPWARD_DYLIB, "@rpath/Upward.framework/Upward"),
        ]);

        assert_eq!(
            requirements.dylibs,
            vec![
                String::from("@rpath/Reexport.framework/Reexport"),
                String::from("@rpath/Upward.framework/Upward"),
                String::from("@rpath/Weak.framework/Weak"),
            ]
        );
    }

    #[test]
    fn de_duplicates_and_sorts_the_dylibs_of_every_slice() {
        let (_directory, path) = executable(&fat_macho_with(&[
            (
                CPU_TYPE_ARM64,
                vec![
                    dylib_command("/usr/lib/libz.1.dylib"),
                    dylib_command("/usr/lib/libbz2.1.0.dylib"),
                ],
            ),
            (
                CPU_TYPE_X86_64,
                vec![
                    dylib_command("/usr/lib/libbz2.1.0.dylib"),
                    dylib_command("/usr/lib/libxml2.2.dylib"),
                ],
            ),
        ]));

        let requirements = requirements(&path).expect("the slices are well formed");

        assert_eq!(
            requirements.architectures,
            vec![Architecture::Arm64, Architecture::X86_64]
        );
        assert_eq!(
            requirements.dylibs,
            vec![
                String::from("/usr/lib/libbz2.1.0.dylib"),
                String::from("/usr/lib/libxml2.2.dylib"),
                String::from("/usr/lib/libz.1.dylib"),
            ]
        );
    }

    #[test]
    fn build_version_minos_wins_over_version_min_macosx() {
        let requirements = thin_requirements(&[
            version_min_macosx_command(packed_version(10, 14, 0)),
            build_version_command(PLATFORM_MACOS, packed_version(10, 15, 3)),
        ]);

        assert_eq!(requirements.minimum_os.as_deref(), Some("10.15.3"));
    }

    #[test]
    fn falls_back_to_version_min_macosx_padding_the_patch_with_zero() {
        let requirements =
            thin_requirements(&[version_min_macosx_command(packed_version(10, 13, 0))]);

        assert_eq!(requirements.minimum_os.as_deref(), Some("10.13.0"));
    }

    #[test]
    fn ignores_a_build_version_that_is_not_for_macos() {
        let with_a_macos_declaration = thin_requirements(&[
            build_version_command(PLATFORM_IOS, packed_version(12, 0, 0)),
            version_min_macosx_command(packed_version(10, 12, 0)),
        ]);
        assert_eq!(
            with_a_macos_declaration.minimum_os.as_deref(),
            Some("10.12.0")
        );

        let alone = thin_requirements(&[build_version_command(
            PLATFORM_IOS,
            packed_version(12, 0, 0),
        )]);
        assert_eq!(alone.minimum_os, None);
    }

    #[test]
    fn reports_no_minimum_os_when_no_command_declares_one() {
        let requirements = thin_requirements(&[]);

        assert_eq!(requirements.minimum_os, None);
        assert!(requirements.dylibs.is_empty());
        assert_eq!(requirements.architectures, vec![Architecture::Arm64]);
    }

    #[test]
    fn reads_the_load_commands_of_a_32_bit_header() {
        let (_directory, path) = executable(&thin_macho_32_with(
            CPU_TYPE_I386,
            &[
                dylib_command("/usr/lib/libSystem.B.dylib"),
                version_min_macosx_command(packed_version(10, 6, 8)),
            ],
        ));

        let requirements = requirements(&path).expect("a 32-bit header holds its commands too");

        assert_eq!(requirements.architectures, vec![Architecture::I386]);
        assert_eq!(
            requirements.dylibs,
            vec![String::from("/usr/lib/libSystem.B.dylib")]
        );
        assert_eq!(requirements.minimum_os.as_deref(), Some("10.6.8"));
    }

    #[test]
    fn the_requirements_serialize_with_the_reported_field_names() {
        let requirements = thin_requirements(&[dylib_command("/usr/lib/libz.1.dylib")]);
        let json = serde_json::to_value(&requirements).expect("the requirements serialize");

        assert_eq!(json["architectures"][0], "arm64");
        assert_eq!(json["dylibs"][0], "/usr/lib/libz.1.dylib");
        assert!(
            json.get("minimum_os").is_none(),
            "an absent minimum OS is omitted, not serialized as null: {json}"
        );
    }

    #[test]
    fn rejects_a_dylib_command_whose_cmdsize_cannot_hold_its_structure() {
        let mut command = dylib_command("/usr/lib/libz.1.dylib");
        command[4..8].copy_from_slice(&8u32.to_le_bytes());
        let (_directory, path) = executable(&thin_macho_with(CPU_TYPE_ARM64, &[command]));

        let error = requirements(&path).expect_err("a dylib command is 24 bytes at least");
        let message = error.to_string();

        assert!(
            matches!(error, Error::MachOInvalidLoadCommand { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("cmdsize 8"),
            "the message must report the declared size: {message}"
        );
        assert!(
            message.contains("24"),
            "the message must report the structure it has to hold: {message}"
        );
    }

    #[test]
    fn rejects_a_dylib_name_offset_outside_the_command() {
        let mut command = dylib_command("/usr/lib/libz.1.dylib");
        command[8..12].copy_from_slice(&0xffffu32.to_le_bytes());
        let (_directory, path) = executable(&thin_macho_with(CPU_TYPE_ARM64, &[command]));

        let error = requirements(&path).expect_err("the name must sit inside the command");
        let message = error.to_string();

        assert!(
            matches!(error, Error::MachOInvalidLoadCommand { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            message.contains("65535"),
            "the message must report the offending offset: {message}"
        );
    }

    #[test]
    fn rejects_a_command_that_runs_past_the_declared_command_size() {
        let mut command = dylib_command("/usr/lib/libz.1.dylib");
        let declared = command.len() as u32;
        command[4..8].copy_from_slice(&(declared + 8).to_le_bytes());
        let (_directory, path) = executable(&thin_macho_with(CPU_TYPE_ARM64, &[command]));

        let error = requirements(&path).expect_err("a command may not exceed sizeofcmds");

        assert!(
            matches!(error, Error::MachOInvalidLoadCommand { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains(&(declared + 8).to_string()),
            "the message must report the declared size: {error}"
        );
    }

    #[test]
    fn rejects_sizeofcmds_past_the_end_of_the_file() {
        let mut bytes = thin_macho_with(CPU_TYPE_ARM64, &[dylib_command("/usr/lib/libz.1.dylib")]);
        let commands = (bytes.len() - HEADER_64_LENGTH as usize) as u32;
        bytes[20..24].copy_from_slice(&(commands + 64).to_le_bytes());
        let length = bytes.len() as u64;
        let (_directory, path) = executable(&bytes);

        let error = requirements(&path).expect_err("the load commands are not in the file");

        assert!(
            matches!(
                error,
                Error::MachOTruncated {
                    needed,
                    found,
                    ..
                } if needed == HEADER_64_LENGTH + u64::from(commands) + 64 && found == length
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_a_truncated_thin_header() {
        let bytes = thin_macho_with(CPU_TYPE_ARM64, &[dylib_command("/usr/lib/libz.1.dylib")]);
        let (_directory, path) = executable(&bytes[..20]);

        let error = requirements(&path).expect_err("a 20 byte file is not a header");

        assert!(
            matches!(
                error,
                Error::MachOTruncated {
                    needed: 32,
                    found: 20,
                    ..
                }
            ),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn rejects_a_universal_binary_whose_slice_is_truncated() {
        let mut bytes =
            fat_macho_with(&[(CPU_TYPE_ARM64, vec![dylib_command("/usr/lib/libz.1.dylib")])]);
        let length = bytes.len();
        bytes.truncate(length - 4);
        let (_directory, path) = executable(&bytes);

        let error = requirements(&path).expect_err("the slice runs past the file");

        assert!(
            matches!(error, Error::MachOInvalidFat { .. }),
            "unexpected error: {error:?}"
        );
        assert!(
            error.to_string().contains("past the file length"),
            "the message must say why the slice is unusable: {error}"
        );
    }
}
