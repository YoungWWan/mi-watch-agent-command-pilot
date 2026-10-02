use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

fn main() {
    compile_protos();
}

fn compile_protos() {
    let proto_dir = Path::new("protos/xiaomi");
    let protos = collect_proto_files(proto_dir);

    for proto in &protos {
        println!("cargo:rerun-if-changed={}", proto.display());
        println!("Proto file {} added to codegen list.", proto.display());
    }

    let out_dir = "src/xiaomi";
    let out_path = Path::new(out_dir);
    if !out_path.exists() {
        println!("Output dir `{}` does not exist, creating…", out_dir);
        fs::create_dir_all(out_path).expect("Error when create out_dir");
    }

    let protoc = protoc_bin_vendored::protoc_bin_path().expect("failed to find vendored protoc");

    unsafe { std::env::set_var("PROTOC", protoc) };

    let descriptor_path = PathBuf::from(out_dir).join("proto_descriptor.bin");

    let mod_path = PathBuf::from(out_dir).join("mod.rs");
    let serde_path = PathBuf::from(out_dir).join("protocol.serde.rs");

    let mut config = prost_build::Config::new();
    config.file_descriptor_set_path(&descriptor_path);
    config.compile_well_known_types();
    config.extern_path(".google.protobuf", "::pbjson_types");
    config.out_dir(out_dir);

    config.compile_protos(&protos, &[proto_dir]).unwrap();

    let descriptor_set = std::fs::read(descriptor_path.clone()).unwrap();
    let _ = pbjson_build::Builder::new()
        .out_dir(out_dir)
        .register_descriptors(&descriptor_set)
        .unwrap()
        .build(&["."]);

    let mut mod_file = std::fs::File::create(mod_path).expect("create failed");
    mod_file
        .write_all("#[path = \"protocol.serde.rs\"]\npub mod protocol;\n".as_bytes())
        .expect("write failed");

    let mut file = OpenOptions::new().append(true).open(&serde_path).unwrap();
    let _ = file.seek(SeekFrom::Start(0));
    let _ = file.write_all(b"\ninclude!(\"protocol.rs\");");

    let _ = std::fs::remove_file(descriptor_path);
}

fn collect_proto_files(dir: &Path) -> Vec<PathBuf> {
    let mut protos = Vec::new();
    collect_proto_files_recursive(dir, &mut protos);
    protos
}

fn collect_proto_files_recursive(dir: &Path, protos: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_proto_files_recursive(&path, protos);
            } else if path.extension().map(|ext| ext == "proto").unwrap_or(false) {
                protos.push(path);
            }
        }
    }
}
