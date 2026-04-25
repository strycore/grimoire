// Force a rebuild whenever the embedded spells directory changes.
// `include_dir!` snapshots files at build time and won't pick up new or
// modified spells without this.
fn main() {
    println!("cargo:rerun-if-changed=spells");
}
