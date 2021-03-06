use cpp_ffi_data::{IndirectionChange, CppAndFfiMethod, CppFfiArgumentMeaning};
use cpp_ffi_generator::CppFfiHeaderData;
use cpp_method::ReturnValueAllocationPlace;
use cpp_type::{CppTypeIndirection, CppTypeBase};
use errors::{Result, ChainErr, unexpected};
use file_utils::{PathBufWithAdded, create_dir_all, create_file, path_to_str};
use log;
use string_utils::JoinWithString;
use utils::MapIfOk;

use std::path::PathBuf;

/// Generates C++ code for the C wrapper library.
pub struct CppCodeGenerator {
  /// Library name
  lib_name: String,
  /// Uppercase library name (for optimization)
  lib_name_upper: String,
  /// Path to the directory where the library is generated
  lib_path: ::std::path::PathBuf,

  is_shared: bool,
  cpp_libs: Vec<String>,
}

impl CppCodeGenerator {
  /// Creates a generator for a library.
  /// lib_name: library name
  /// lib_path: path to the directory where the library is generated
  pub fn new(lib_name: String,
             lib_path: ::std::path::PathBuf,
             is_shared: bool,
             cpp_libs: Vec<String>)
             -> Self {
    CppCodeGenerator {
      lib_name: lib_name.clone(),
      lib_name_upper: lib_name.to_uppercase(),
      lib_path: lib_path,
      is_shared: is_shared,
      cpp_libs: cpp_libs,
    }
  }

  /// Generates function name, return type and arguments list
  /// as it appears in both function declaration and implementation.
  fn function_signature(&self, method: &CppAndFfiMethod) -> Result<String> {
    let mut arg_texts = Vec::new();
    for arg in &method.c_signature.arguments {
      arg_texts.push(try!(arg.to_cpp_code()));
    }
    let name_with_args = format!("{}({})", method.c_name, arg_texts.join(", "));
    let return_type = &method.c_signature.return_type.ffi_type;
    let r = if let CppTypeBase::FunctionPointer { .. } = return_type.base {
      try!(return_type.to_cpp_code(Some(&name_with_args)))
    } else {
      format!("{} {}", try!(return_type.to_cpp_code(None)), name_with_args)
    };
    Ok(r)
  }

  /// Generates method declaration for the header.
  fn function_declaration(&self, method: &CppAndFfiMethod) -> Result<String> {
    Ok(format!("{}_EXPORT {};\n",
               self.lib_name_upper,
               try!(self.function_signature(method))))
  }

  /// Wraps expression returned by the original method to
  /// convert it to return type of the FFI method.
  fn convert_return_type(&self, method: &CppAndFfiMethod, expression: String) -> Result<String> {
    let mut result = expression;
    match method.c_signature.return_type.conversion {
      IndirectionChange::NoChange => {}
      IndirectionChange::ValueToPointer => {
        match method.allocation_place {
          ReturnValueAllocationPlace::Stack => {
            return Err(unexpected("stack allocated wrappers are expected to return void").into());
          }
          ReturnValueAllocationPlace::NotApplicable => {
            return Err(unexpected("ValueToPointer conflicts with NotApplicable").into());
          }
          ReturnValueAllocationPlace::Heap => {
            // constructors are said to return values in parse result,
            // but in reality we use `new` which returns a pointer,
            // so no conversion is necessary for constructors.
            if !method.cpp_method.is_constructor() {
              result = format!("new {}({})",
                               try!(method.cpp_method.return_type.base.to_cpp_code(None)),
                               result);
            }
          }
        }
      }
      IndirectionChange::ReferenceToPointer => {
        result = format!("&{}", result);
      }
      IndirectionChange::QFlagsToUInt => {
        result = format!("uint({})", result);
      }
    }

    if method.allocation_place == ReturnValueAllocationPlace::Stack &&
       !method.cpp_method.is_constructor() {
      if let Some(arg) = method.c_signature
        .arguments
        .iter()
        .find(|x| x.meaning == CppFfiArgumentMeaning::ReturnValue) {
        result = format!("new({}) {}({})",
                         arg.name,
                         try!(method.cpp_method.return_type.base.to_cpp_code(None)),
                         result);
      }
    }
    Ok(result)
  }

  /// Generates code for values passed to the original C++ method.
  fn arguments_values(&self, method: &CppAndFfiMethod) -> Result<String> {
    let mut filled_arguments = vec![];
    for (i, cpp_argument) in method.cpp_method.arguments.iter().enumerate() {
      if let Some(c_argument) = method.c_signature
        .arguments
        .iter()
        .find(|x| x.meaning == CppFfiArgumentMeaning::Argument(i as i8)) {
        let mut result = c_argument.name.clone();
        match c_argument.argument_type
          .conversion {
          IndirectionChange::ValueToPointer |
          IndirectionChange::ReferenceToPointer => result = format!("*{}", result),
          IndirectionChange::NoChange => {}
          IndirectionChange::QFlagsToUInt => {
            let type_text = if cpp_argument.argument_type.indirection == CppTypeIndirection::Ref &&
                               cpp_argument.argument_type.is_const {
              let mut fake_type = cpp_argument.argument_type.clone();
              fake_type.is_const = false;
              fake_type.indirection = CppTypeIndirection::None;
              try!(fake_type.to_cpp_code(None))
            } else {
              try!(cpp_argument.argument_type.to_cpp_code(None))
            };
            result = format!("{}({})", type_text, result);
          }
        }
        filled_arguments.push(result);
      } else {
        panic!("Error: no positional argument found\n{:?}", method);
      }
    }
    Ok(filled_arguments.into_iter().join(", "))
  }

  /// Generates code for the value returned by the FFI method.
  #[cfg_attr(feature="clippy", allow(collapsible_if))]
  fn returned_expression(&self, method: &CppAndFfiMethod) -> Result<String> {
    let result = if method.cpp_method.is_destructor() {
      if let Some(arg) = method.c_signature
        .arguments
        .iter()
        .find(|x| x.meaning == CppFfiArgumentMeaning::This) {
        format!("{}_call_destructor({})", self.lib_name, arg.name)
      } else {
        return Err(unexpected("no this arg in destructor").into());
      }
    } else {
      let result_without_args = if let Some(info) = method.cpp_method
        .class_info_if_constructor() {
        let class_type = &info.class_type;
        match method.allocation_place {
          ReturnValueAllocationPlace::Stack => {
            if let Some(arg) = method.c_signature
              .arguments
              .iter()
              .find(|x| x.meaning == CppFfiArgumentMeaning::ReturnValue) {
              format!("new({}) {}", arg.name, try!(class_type.to_cpp_code()))
            } else {
              return Err(unexpected(format!("return value argument not found\n{:?}", method))
                .into());
            }
          }
          ReturnValueAllocationPlace::Heap => format!("new {}", try!(class_type.to_cpp_code())),
          ReturnValueAllocationPlace::NotApplicable => {
            return Err(unexpected("NotApplicable in constructor").into());
          }
        }
      } else {
        let scope_specifier = if let Some(ref class_membership) = method.cpp_method
          .class_membership {
          if class_membership.is_static {
            format!("{}::", try!(class_membership.class_type.to_cpp_code()))
          } else {
            if let Some(arg) = method.c_signature
              .arguments
              .iter()
              .find(|x| x.meaning == CppFfiArgumentMeaning::This) {
              format!("{}->", arg.name)
            } else {
              return Err(unexpected("no this arg in non-static method").into());
            }
          }
        } else {
          "".to_string()
        };
        let template_args = match method.cpp_method.template_arguments_values {
          Some(ref args) => {
            let mut texts = Vec::new();
            for arg in args {
              texts.push(try!(arg.to_cpp_code(None)));
            }
            format!("<{}>", texts.join(", "))
          }
          None => String::new(),
        };
        format!("{}{}{}",
                scope_specifier,
                method.cpp_method.name,
                template_args)
      };
      format!("{}({})",
              result_without_args,
              try!(self.arguments_values(method)))
    };
    self.convert_return_type(method, result)
  }

  /// Generates body of the FFI method implementation.
  fn source_body(&self, method: &CppAndFfiMethod) -> Result<String> {
    if method.cpp_method.is_destructor() &&
       method.allocation_place == ReturnValueAllocationPlace::Heap {
      if let Some(arg) = method.c_signature
        .arguments
        .iter()
        .find(|x| x.meaning == CppFfiArgumentMeaning::This) {
        Ok(format!("delete {};\n", arg.name))
      } else {
        panic!("Error: no this argument found\n{:?}", method);
      }
    } else {
      Ok(format!("{}{};\n",
                 if method.c_signature.return_type.ffi_type.is_void() {
                   ""
                 } else {
                   "return "
                 },
                 try!(self.returned_expression(&method))))
    }
  }

  /// Generates implementation of the FFI method for the source file.
  fn function_implementation(&self, method: &CppAndFfiMethod) -> Result<String> {
    Ok(format!("{} {{\n  {}}}\n\n",
               try!(self.function_signature(method)),
               try!(self.source_body(&method))))
  }

  /// Generates main files and directories of the library.
  pub fn generate_template_files(&self,
                                 include_directives: &[PathBuf],
                                 include_directories: &[String],
                                 framework_directories: &[String],
                                 cpp_compiler_flags: &[String])
                                 -> Result<()> {
    let name_upper = self.lib_name.to_uppercase();
    let cmakelists_path = self.lib_path.with_added("CMakeLists.txt");
    let mut cmakelists_file = try!(create_file(&cmakelists_path));

    let mut all_cpp_flags = Vec::from(cpp_compiler_flags);
    for dir in framework_directories {
      all_cpp_flags.push(format!("-F\"{}\"", dir));
    }
    let all_cpp_flags_text = all_cpp_flags.iter().map(|x| x.replace("\"", "\\\"")).join(" ");
    try!(cmakelists_file.write(format!(include_str!("../templates/c_lib/CMakeLists.txt"),
                                       lib_name_lowercase = &self.lib_name,
                                       lib_name_uppercase = name_upper,
                                       include_directories = include_directories.into_iter()
                                         .map(|x| format!("\"{}\"", x.replace(r"\", r"\\")))
                                         .join(" "),
                                       library_type =
                                         if self.is_shared { "SHARED" } else { "STATIC" },
                                       target_link_libraries = if self.is_shared {
                                         format!("target_link_libraries({} {})",
                                                 &self.lib_name,
                                                 self.cpp_libs.join(" "))
                                       } else {
                                         String::new()
                                       },
                                       cxx_flags = all_cpp_flags_text)));
    let src_dir = self.lib_path.with_added("src");
    try!(create_dir_all(&src_dir));

    let include_dir = self.lib_path.with_added("include");
    try!(create_dir_all(&include_dir));
    let exports_file_path = include_dir.with_added(format!("{}_exports.h", &self.lib_name));
    let mut exports_file = try!(create_file(&exports_file_path));
    try!(exports_file.write(format!(include_str!("../templates/c_lib/exports.h"),
                                    lib_name_uppercase = name_upper)));

    let include_directives_code = try!(include_directives.map_if_ok(|d| -> Result<_> {
        Ok(format!("#include \"{}\"", try!(path_to_str(d))))
      }))
      .join("\n");

    let global_file_path = include_dir.with_added(format!("{}_global.h", &self.lib_name));
    let mut global_file = try!(create_file(&global_file_path));
    try!(global_file.write(format!(include_str!("../templates/c_lib/global.h"),
                                   lib_name_lowercase = &self.lib_name,
                                   lib_name_uppercase = name_upper,
                                   include_directives_code = include_directives_code)));
    Ok(())
  }

  pub fn generate_files(&self, data: &[CppFfiHeaderData]) -> Result<()> {
    try!(self.generate_all_headers_file(data.iter().map(|x| &x.include_file)));
    for item in data {
      try!(self.generate_one(item).chain_err(|| "C++ code generator failed"));
    }
    Ok(())
  }

  /// Generates the header file that includes all other headers of the library.
  fn generate_all_headers_file<'a, I: Iterator<Item = &'a String>>(&self, names: I) -> Result<()> {
    let mut h_path = self.lib_path.clone();
    h_path.push("include");
    h_path.push(format!("{}.h", &self.lib_name));
    let mut all_header_file = try!(create_file(&h_path));
    try!(all_header_file.write(format!("#ifndef {0}_H\n#define {0}_H\n\n", &self.lib_name_upper)));
    for name in names {
      try!(all_header_file.write(format!("#include \"{}_{}.h\"\n", &self.lib_name, name)));
    }
    try!(all_header_file.write(format!("#endif // {}_H\n", &self.lib_name_upper)));
    Ok(())
  }

  /// Generates a header file and a source file for a portion of data
  /// corresponding to a header file of original C++ library.
  fn generate_one(&self, data: &CppFfiHeaderData) -> Result<()> {
    let ffi_include_file = format!("{}_{}.h", &self.lib_name, data.include_file_base_name);

    let cpp_path = self.lib_path
      .with_added("src")
      .with_added(format!("{}_{}.cpp", &self.lib_name, data.include_file_base_name));
    log::noisy(format!("Generating source file: {:?}", cpp_path));

    let h_path = self.lib_path.with_added("include").with_added(&ffi_include_file);
    log::noisy(format!("Generating header file: {:?}", h_path));

    let mut cpp_file = try!(create_file(&cpp_path));
    let mut h_file = try!(create_file(&h_path));

    try!(cpp_file.write(format!("#include \"{}\"\n\n", ffi_include_file)));
    let include_guard_name = ffi_include_file.replace(".", "_").to_uppercase();
    try!(h_file.write(format!("#ifndef {}\n#define {}\n\n",
                              include_guard_name,
                              include_guard_name)));

    try!(h_file.write(format!("#include \"{}_global.h\"\n\n", &self.lib_name)));

    try!(h_file.write("extern \"C\" {\n\n"));

    for method in &data.methods {
      try!(h_file.write(try!(self.function_declaration(method))));
      try!(cpp_file.write(try!(self.function_implementation(method))));
    }

    try!(h_file.write("\n} // extern \"C\"\n\n"));

    try!(h_file.write(format!("#endif // {}\n", include_guard_name)));
    Ok(())
  }
}
