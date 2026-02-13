fn main() {
    prost_build::compile_protos(&["proto/gtfs-realtime.proto"], &["proto/"]).unwrap();
}
