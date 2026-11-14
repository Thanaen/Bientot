//! A `.bientot-plugin` file as it may arrive from anywhere: no file panics the reader, and a
//! package that reads has a valid manifest, a module and the texts of each locale it declares.

#![no_main]

use bientot_plugin_api::Language;
use bientot_plugins::package::Package;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let package = match Package::read(bytes, &["Open settings"]) {
        Ok(package) => package,
        Err(error) => {
            error.message(Language::En);
            error.message(Language::Fr);
            return;
        }
    };
    for locale in &package.manifest.locales {
        assert!(
            package.locales.contains_key(locale),
            "the texts of {locale:?} are read"
        );
    }
    package.module();
    assert_eq!(package.hashes().len(), package.files.len());
    package.list_hash();
});
