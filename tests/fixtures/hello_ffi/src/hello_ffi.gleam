//// SPDX-License-Identifier: MIT OR Apache-2.0
//// The zero-dependency fixture application.
////
//// Every observable thing this program does happens in `hello_ffi_ffi.erl`.
//// Without `gleam_stdlib` there is no `io` module, so the Gleam half is one
//// external declaration and the `main` that Gleam's generated
//// `hello_ffi@@main` entry point calls.

@external(erlang, "hello_ffi_ffi", "main")
fn ffi_main() -> Nil

/// Prints the plain arguments, the `priv/greeting.txt` contents and the
/// working directory, then halts with the first argument as the exit code.
pub fn main() -> Nil {
  ffi_main()
}
