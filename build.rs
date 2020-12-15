use std::{
    env,
    ffi::OsString,
    fs,
    io,
    io::{ Error, ErrorKind },
    path::{ Path, PathBuf },
    process::{ exit, Command },
};

#[cfg(target_family = "unix")]
use std::{
    os::unix::ffi::OsStrExt,
    ffi::OsStr,
};

#[cfg(target_family = "windows")]
use std::os::windows::ffi::OsStringExt;

#[allow(dead_code)]
struct InstallationPaths {
    r_home: PathBuf,
    include: PathBuf,
    library: PathBuf,
}

#[allow(dead_code)]
#[derive(Debug)]
struct RVersionInfo {
    major: String,
    minor: String,
    patch: String,
    devel: String,
}


// frustratingly, something like the following does not exist in an
// OS-independent way in Rust
#[cfg(target_family = "unix")]
fn byte_array_to_os_string(bytes: &[u8]) -> OsString {
    let os_str = OsStr::from_bytes(bytes);
    os_str.to_os_string()
}

// convert bytes to wide-encoded characters on Windows
// from: https://stackoverflow.com/a/40456495/4975218
#[cfg(target_family = "windows")]
fn wide_from_console_string(bytes: &[u8]) -> Vec<u16> {
    assert!(bytes.len() < std::i32::MAX as usize);
    let mut wide;
    let mut len;
    unsafe {
        let cp = kernel32::GetConsoleCP();
        len = kernel32::MultiByteToWideChar(cp, 0, bytes.as_ptr() as *const i8, bytes.len() as i32, std::ptr::null_mut(), 0);
        wide = Vec::with_capacity(len as usize);
        len = kernel32::MultiByteToWideChar(cp, 0, bytes.as_ptr() as *const i8, bytes.len() as i32, wide.as_mut_ptr(), len);
        wide.set_len(len as usize);
    }
    wide
}

#[cfg(target_family = "windows")]
fn byte_array_to_os_string(bytes: &[u8]) -> OsString {
    // first, use Windows API to convert to wide encoded
    let wide = wide_from_console_string(bytes);
    // then, use `std::os::windows::ffi::OsStringExt::from_wide()`
    OsString::from_wide(&wide)
}

fn probe_r_paths() -> io::Result<InstallationPaths> {
    // First we locate the R home
    let r_home = match env::var_os("R_HOME") {
        // If the environment variable R_HOME is set we use it
        Some(s) => PathBuf::from(s),

        // Otherwise, we try to execute `R` to find `R_HOME`. Note that this is
        // discouraged, see Section 1.6 of "Writing R Extensions"
        // https://cran.r-project.org/doc/manuals/r-release/R-exts.html#Writing-portable-packages
        _ => {
            let rout = Command::new("R")
                .args(&[
                    "-s",
                    "-e",
                    r#"cat(normalizePath(R.home()))"#
                ])
                .output()?;

            let rout = byte_array_to_os_string(&rout.stdout);
            if !rout.is_empty() {
                PathBuf::from(rout)
            } else {
                return Err(Error::new(ErrorKind::Other, "Cannot find R home."));
            }
        }
    };

    // Now the library location. On Windows, it depends on the target architecture
    let pkg_target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let library = if cfg!(target_os = "windows") {
        if pkg_target_arch == "x86_64" {
            Path::new(&r_home)
                .join("bin")
                .join("x64")
        } else if pkg_target_arch == "x86" {
            Path::new(&r_home)
                .join("bin")
                .join("i386")
        } else {
            panic!("Unknown architecture")
        }
    } else {
        Path::new(&r_home).join("lib")
    };

    // Finally the include location. It may or may not be located under R home
    let include = match env::var_os("R_INCLUDE_DIR") {
        // If the environment variable R_INCLUDE_DIR is set we use it
        Some(s) => PathBuf::from(s),

        // Otherwise, we try to execute `R` to find the include dir. Here,
        // we're using the R home we found earlier, to make sure we're consistent.
        _ => {
            let r_binary = if cfg!(target_os = "windows") {
                Path::new(&library)
                    .join("R.exe")
            } else {
                Path::new(&r_home)
                    .join("bin")
                    .join("R")
            };

            let out = Command::new(&r_binary)
                .args(&[
                    "-s",
                    "-e",
                    r#"cat(normalizePath(R.home('include')))"#
                ])
                .output()?;

            // if there are any errors we print them out, helps with debugging
            if !out.stderr.is_empty() {
                println!("> {}",
                    byte_array_to_os_string(&out.stderr)
                    .as_os_str()
                    .to_string_lossy()
                );
            }

            let rout = byte_array_to_os_string(&out.stdout);
            if !rout.is_empty() {
                PathBuf::from(rout)
            } else {
                return Err(Error::new(ErrorKind::Other, "Cannot find R include."));
            }
        }
    };

    Ok(InstallationPaths {
        r_home,
        include,
        library,
    })
}


fn get_r_version_strings(r_paths: &InstallationPaths) -> io::Result<RVersionInfo> {
    let r_binary = if cfg!(target_os = "windows") {
        Path::new(&r_paths.library)
            .join("R.exe")
    } else {
        Path::new(&r_paths.r_home)
            .join("bin")
            .join("R")
    };

    let out = Command::new(&r_binary)
        .args(&[
            "-s",
            "-e",
            r#"v <- strsplit(R.version$minor, '.', fixed = TRUE)[[1]];devel <- isTRUE(grepl('devel', R.version$status, fixed = TRUE));cat(R.version$major, v[1], paste0(v[2:length(v)], collapse = '.'), devel, sep = '\n')"#
        ])
        .output()?;

    let out = byte_array_to_os_string(&out.stdout)
        .as_os_str()
        .to_string_lossy()
        .into_owned();
    let mut lines = out.lines();

    let major = match lines.next() {
        Some(line) => line.to_string(),
        _ => return Err(Error::new(ErrorKind::Other, "Cannot find R major version")),
    };

    let minor = match lines.next() {
        Some(line) => line.to_string(),
        _ => return Err(Error::new(ErrorKind::Other, "Cannot find R minor version")),
    };

    let patch = match lines.next() {
        Some(line) => line.to_string(),
        _ => return Err(Error::new(ErrorKind::Other, "Cannot find R patch level")),
    };

    let devel = match lines.next() {
        Some(line) => if line == "TRUE" {
            "-devel".to_string()
        } else {
            "".to_string()
        },
        _ => return Err(Error::new(ErrorKind::Other, "Cannot find R development status")),
    };

    Ok(RVersionInfo {
        major,
        minor,
        patch,
        devel,
    })
}

#[cfg(feature = "default")]
/// Generate bindings by calling bindgen.
fn generate_bindings(r_paths: &InstallationPaths) {
    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let mut bindgen_builder = bindgen::Builder::default()
        // These constants from libm break bindgen.
        .blacklist_item("FP_NAN")
        .blacklist_item("FP_INFINITE")
        .blacklist_item("FP_ZERO")
        .blacklist_item("FP_SUBNORMAL")
        .blacklist_item("FP_NORMAL")
        // The input header we would like to generate
        // bindings for.
        .header("wrapper.h")
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks));

    let target = env::var("TARGET").expect("Could not get the target triple");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    println!("Generating bindings for target: {}, os: {}, architecture: {}", target, target_os, target_arch);
    // Point to the correct headers
    bindgen_builder = bindgen_builder.clang_args(&[
        format!("-I{}", r_paths.include.display()),
        format!("--target={}", target)
    ]);

    // allow injection of an alternative include path to libclang
    if let Some(alt_include) = env::var_os("LIBRSYS_LIBCLANG_INCLUDE_PATH") {
        bindgen_builder = bindgen_builder.clang_arg(
            format!("-I{}", PathBuf::from(alt_include).display()),
        );
    }

    // disable layout tests for windows x86
    if target_os == "windows" && target_arch == "x86" {
        bindgen_builder = bindgen_builder.layout_tests(false);
    }

    // Finish the builder and generate the bindings.
    let bindings = bindgen_builder
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Extract the version number from the R headers.
    let version_matcher = regex::Regex::new(r"pub const R_VERSION ?: ?u32 = (\d+)").unwrap();
    if let Some(version) = version_matcher.captures(bindings.to_string().as_str()) {
        let version = version.get(1).unwrap().as_str().parse::<u32>().unwrap();
        println!("cargo:r_version={}", version);
    } else {
        panic!("failed to find R_VERSION");
    }

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings to default output path!");

    // Also write the bindings to a folder specified by LIBRSYS_BINDINGS_OUTPUT_PATH, if defined
    if let Some(alt_target) = env::var_os("LIBRSYS_BINDINGS_OUTPUT_PATH") {
        let out_path = PathBuf::from(alt_target);
        // if folder doesn't exist, try to create it
        if !out_path.exists() {
            fs::create_dir(&out_path)
                .expect(&format!("Couldn't create output directory for bindings: {}", out_path.display()));
        }

        let version_info = get_r_version_strings(r_paths).expect("Could not obtain R version");
        let out_file = out_path.join(
                format!(
                    "bindings-{}-{}-R{}.{}{}.rs",
                    target_os, target_arch, version_info.major, version_info.minor, version_info.devel
                )
            );

        bindings
            .write_to_file(&out_file)
            .expect(&format!("Couldn't write bindings: {}", out_file.display()));
    }
}


#[allow(dead_code)]
/// Retrieve bindings from cache, if available. Errors out otherwise.
fn retrieve_prebuild_bindings(r_paths: &InstallationPaths) {
    let version_info = get_r_version_strings(r_paths).expect("Could not obtain R version");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let bindings_path = PathBuf::from(
        env::var_os("LIBRSYS_BINDINGS_PATH")
        .unwrap_or(OsString::from("bindings"))
    );
    
    // we try a few different file names, from more specific to less specific
    let bindings_file_full = PathBuf::from(
        format!(
            "bindings-{}-{}-R{}.{}{}.rs",
            target_os, target_arch, version_info.major, version_info.minor, version_info.devel
        )
    );
    let bindings_file_novers = PathBuf::from(
        format!("bindings-{}-{}.rs", target_os, target_arch)
    );

    let mut from = bindings_path.join(bindings_file_full);
    if !from.exists() {
        from = bindings_path.join(bindings_file_novers);
        if !from.exists() {
            panic!(
                format!(
                    "Cannot find libR-sys bindings file for R {}.{}.{}{} on {} in {}. Consider compiling with default features enabled.",
                    version_info.major, version_info.minor, version_info.patch, version_info.devel, target_os, bindings_path.display()
                )
            )
        } else {
            println!(
                "Warning: using generic {}-{} libR-sys bindings. These may not work for R {}.{}.{}{}.",
                target_os, target_arch, version_info.major, version_info.minor, version_info.patch, version_info.devel
            );
        }
    }

    fs::copy(
        from,
        PathBuf::from(env::var_os("OUT_DIR").unwrap())
            .join("bindings.rs")
    ).expect("No precomputed bindings available!");
}

fn main() {
    let r_paths = probe_r_paths();

    let r_paths = match r_paths {
        Ok(result) => result,
        Err(error) => {
            println!("Problem locating local R install: {:?}", error);
            exit(1);
        }
    };

    println!("cargo:rustc-env=R_HOME={}", r_paths.r_home.display());
    println!("cargo:r_home={}", r_paths.r_home.display()); // Becomes DEP_R_R_HOME for clients
    // make sure cargo links properly against library
    println!("cargo:rustc-link-search={}", r_paths.library.display());
    println!("cargo:rustc-link-lib=dylib=R");

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=wrapper.h");

    #[cfg(feature = "default")]
        generate_bindings(&r_paths);
    #[cfg(not(feature = "default"))]
        retrieve_prebuild_bindings(&r_paths);
}
