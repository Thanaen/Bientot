//! A damaged transfer file: bytes of a real export overwritten at the positions the fuzzer picks.
//! The export carries a `settings.json`. Inspecting it, as the import does before it asks to
//! confirm, never panics.

#![no_main]

use std::fs;
use std::path::PathBuf;
use std::sync::OnceLock;

use bientot_store::data_directory::DataDirectory;
use bientot_store::{database, transfer};
use jiff::Timestamp;
use libfuzzer_sys::fuzz_target;

struct Scratch {
    export: Vec<u8>,
    file: PathBuf,
}

fn scratch() -> &'static Scratch {
    static SCRATCH: OnceLock<Scratch> = OnceLock::new();
    SCRATCH.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("bientot-fuzz-transfer-{}", std::process::id()));
        let directory = DataDirectory::named(&root.join("data")).expect("the data folder is named");
        let ledger = database::open(&directory.database_file()).expect("the ledger opens");
        database::migrate(&ledger).expect("the ledger migrates");
        drop(ledger);
        fs::write(directory.settings_file(), r#"{"schemaVersion":1}"#)
            .expect("the settings are written");
        let file = root.join("export.bientot");
        transfer::export(&directory, &file, Timestamp::UNIX_EPOCH).expect("the ledger exports");
        let export = fs::read(&file).expect("the export reads");
        Scratch { export, file }
    })
}

fuzz_target!(|patches: Vec<(u32, u8)>| {
    let scratch = scratch();
    let mut bytes = scratch.export.clone();
    for (offset, byte) in patches {
        let index = offset as usize % bytes.len();
        bytes[index] = byte;
    }
    fs::write(&scratch.file, &bytes).expect("the scratch file is written");
    let _ = transfer::inspect(&scratch.file);
});
