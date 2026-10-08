//! PrismOS developer tool (host, stable Rust).
//!
//! ROLE:
//! Single host binary for the whole Phase-1 workflow, with NO nightly
//! Cargo features required:
//!
//!   - `cargo run-uefi`  -> build kernel + create UEFI image + boot QEMU
//!   - `cargo run-bios`  -> build kernel + create BIOS image + boot QEMU
//!   - `cargo run -- build` -> only build images, do not boot
//!
//! WHY A SUBPROCESS AND NOT ARTIFACT-DEPENDENCIES:
//! Cargo artifact-dependencies (`artifact = "bin"`) still need nightly
//! (`-Z bindeps`). That would force every contributor onto nightly, which
//! the Rust community rightly dislikes. A 30-line subprocess keeps us on
//! stable and makes the pipeline obvious: compile kernel, wrap it in a
//! bootloader image, boot it.
//!
//! LAYOUT:
//!   target/x86_64-unknown-none/debug/kernel  (kernel ELF)
//!   target/images/uefi.img                   (GPT + ESP, primary)
//!   target/images/bios.img                   (legacy fallback)

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// OVMF firmware candidates for UEFI boot (Arch `edk2-ovmf` package first).
const OVMF_BIOS_PATHS: &[&str] = &[
    "/usr/share/edk2/x64/OVMF.4m.fd",
    "/usr/share/edk2-ovmf/x64/OVMF.4m.fd",
    "/usr/share/OVMF/OVMF_CODE_4M.fd",
];

/// Where the kernel ELF lands after `cargo build -p kernel`.
fn kernel_elf_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("x86_64-unknown-none")
        .join("debug")
        .join("kernel")
}

/// Where we write bootable images (NOT in OUT_DIR: they must survive and be
/// inspectable with `qemu-img info` / `fdisk -l` by contributors).
fn images_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("images")
}

/// Step 1: compile the kernel for bare metal.
///
/// We shell out to `cargo build` so the top-level build stays on stable.
/// `--target x86_64-unknown-none` gives us a freestanding ELF with no OS.
fn build_kernel() {
    println!("[prism-os] building kernel (x86_64-unknown-none)...");
    let status = Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("kernel")
        .arg("--target")
        .arg("x86_64-unknown-none")
        .status()
        .expect("failed to spawn `cargo build -p kernel`");
    assert!(
        status.success(),
        "kernel build failed; fix kernel errors before creating images"
    );

    let elf = kernel_elf_path();
    assert!(
        elf.exists(),
        "kernel ELF missing at {} after successful build",
        elf.display()
    );
}

/// Step 2: wrap the kernel ELF in UEFI + BIOS disk images.
///
/// Uses the audited `bootloader` crate as a library. Our own Stage-2 loader
/// is future work behind an RFC; reusing this crate keeps Phase 1 small.
fn build_images() -> (PathBuf, PathBuf) {
    build_kernel();

    let elf = kernel_elf_path();
    let dir = images_dir();
    std::fs::create_dir_all(&dir).expect("failed to create target/images");

    let uefi_path = dir.join("uefi.img");
    bootloader::UefiBoot::new(&elf)
        .create_disk_image(&uefi_path)
        .expect("failed to create UEFI disk image");
    println!("[prism-os] UEFI image: {}", uefi_path.display());

    let bios_path = dir.join("bios.img");
    bootloader::BiosBoot::new(&elf)
        .create_disk_image(&bios_path)
        .expect("failed to create BIOS disk image");
    println!("[prism-os] BIOS image: {}", bios_path.display());

    (uefi_path, bios_path)
}

/// Pick the first OVMF firmware file present on this machine.
fn find_ovmf() -> Option<PathBuf> {
    OVMF_BIOS_PATHS
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

/// Boot the UEFI image with OVMF firmware.
fn boot_uefi(image: &Path) -> ExitStatus {
    let ovmf = find_ovmf().expect(
        "OVMF firmware not found. On Arch: `sudo pacman -S edk2-ovmf`. \
         Expected one of: /usr/share/edk2/x64/OVMF.4m.fd",
    );
    println!("[prism-os] booting UEFI: {}", image.display());

    Command::new("qemu-system-x86_64")
        .arg("-bios")
        .arg(&ovmf)
        .arg("-drive")
        .arg(format!("format=raw,file={}", image.display()))
        .arg("-serial")
        .arg("stdio")
        .arg("-display")
        .arg("none")
        .arg("-m")
        .arg("512M")
        .arg("-smp")
        .arg("1")
        // No `-enable-kvm` on purpose: unavailable in most containers/VMs.
        .status()
        .expect("failed to launch qemu-system-x86_64 for UEFI boot")
}

/// Boot the legacy BIOS image (no OVMF needed).
fn boot_bios(image: &Path) -> ExitStatus {
    println!("[prism-os] booting BIOS: {}", image.display());

    Command::new("qemu-system-x86_64")
        .arg("-drive")
        .arg(format!("format=raw,file={}", image.display()))
        .arg("-serial")
        .arg("stdio")
        .arg("-display")
        .arg("none")
        .arg("-m")
        .arg("512M")
        .arg("-smp")
        .arg("1")
        .status()
        .expect("failed to launch qemu-system-x86_64 for BIOS boot")
}

fn print_usage(program: &str) {
    eprintln!("Usage:");
    eprintln!("  {program} --uefi   Build + boot UEFI image (default, recommended)");
    eprintln!("  {program} --bios   Build + boot BIOS image (fallback)");
    eprintln!("  {program} build    Only build both images, do not boot");
}

fn main() {
    let program = std::env::args().next().unwrap_or_else(|| "prism-os".into());
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Default to UEFI: it is our primary, modern target.
    let mode = args.first().map(String::as_str).unwrap_or("--uefi");

    match mode {
        "build" => {
            build_images();
        }
        "--uefi" => {
            let (uefi, _) = build_images();
            let status = boot_uefi(&uefi);
            std::process::exit(status.code().unwrap_or(1));
        }
        "--bios" => {
            let (_, bios) = build_images();
            let status = boot_bios(&bios);
            std::process::exit(status.code().unwrap_or(1));
        }
        "--help" | "-h" => print_usage(&program),
        other => {
            eprintln!("[prism-os] unknown option: {other}");
            print_usage(&program);
            std::process::exit(2);
        }
    }
}
