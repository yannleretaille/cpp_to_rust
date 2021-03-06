use errors::Result;
use file_utils::path_to_str;
use utils::{is_msvc, add_env_path_item, run_command};

use std::process::Command;
use std::path::PathBuf;

pub struct CppLibBuilder<'a> {
  pub cmake_source_dir: &'a PathBuf,
  pub build_dir: &'a PathBuf,
  pub install_dir: &'a PathBuf,
  pub num_jobs: i32,
  pub linker_env_library_dirs: Option<&'a Vec<PathBuf>>,
  pub pipe_output: bool,
}

impl<'a> CppLibBuilder<'a> {
  pub fn run(self) -> Result<()> {
    let mut cmake_command = Command::new("cmake");
    cmake_command.arg(self.cmake_source_dir)
      .arg(format!("-DCMAKE_INSTALL_PREFIX={}",
                   try!(path_to_str(&self.install_dir))))
      .current_dir(self.build_dir);
    if is_msvc() {
      cmake_command.arg("-G").arg("NMake Makefiles");
      // Rust always links to release version of MSVC runtime, so
      // link will fail if C library is built in debug mode
      cmake_command.arg("-DCMAKE_BUILD_TYPE=Release");
    }
    // TODO: enable release mode on other platforms if cargo is in release mode
    // (maybe build C library in both debug and release in separate folders)
    try!(run_command(&mut cmake_command, false, self.pipe_output));

    let make_command_name = if is_msvc() { "nmake" } else { "make" }.to_string();
    let mut make_args = Vec::new();
    if !is_msvc() {
      // nmake doesn't support multiple jobs
      // TODO: allow to use jom
      make_args.push(format!("-j{}", self.num_jobs));
    }
    make_args.push("install".to_string());
    let mut make_command = Command::new(make_command_name);
    make_command.args(&make_args)
      .current_dir(self.build_dir);
    if let Some(linker_env_library_dirs) = self.linker_env_library_dirs {
      if !linker_env_library_dirs.is_empty() {
        for name in &["LIBRARY_PATH", "LD_LIBRARY_PATH", "LIB"] {
          let value = try!(add_env_path_item(name, (*linker_env_library_dirs).clone()));
          make_command.env(name, value);
        }
      }
    }
    try!(run_command(&mut make_command, false, self.pipe_output));
    Ok(())
  }
}
