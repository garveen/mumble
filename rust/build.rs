fn main() {
    let mut config = prost_build::Config::new();
    config.default_package_filename("mumble_proto");
    config
        .compile_protos(&["proto/mumble.proto"], &["proto/"])
        .expect("Failed to compile Mumble .proto definitions");
}
