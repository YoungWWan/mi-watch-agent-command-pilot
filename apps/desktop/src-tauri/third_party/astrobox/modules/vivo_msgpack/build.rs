#[path = "codegen/mod.rs"]
mod codegen;

fn main() {
    if let Err(err) = codegen::run() {
        panic!("failed to generate vivo msgpack bindings: {err}");
    }
}
