//! What a vault says about itself to Daftar on every device, in the
//! `.daftar.toml` at its root, beside `daftar-cli`'s own sections:
//!
//! ```toml
//! [reader]
//! folded = true
//! ```
//!
//! `reader.folded`: a note opens with its sections folded, each heading
//! saying what is under it (`docs/specs/folded-sections.md`). A file that is
//! not there, or a key that is not, is the default. The port of
//! `VaultSettings.swift`.

use std::path::Path;

use crate::ConfigFile;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VaultSettings {
    pub folded: bool,
}

impl VaultSettings {
    pub const FILE: &'static str = ".daftar.toml";

    pub fn new(text: &str) -> VaultSettings {
        VaultSettings { folded: ConfigFile::new(text).bool("reader.folded").unwrap_or(false) }
    }

    pub fn read(root: &Path) -> VaultSettings {
        std::fs::read_to_string(root.join(Self::FILE)).map_or_default(|text| VaultSettings::new(&text))
    }
}

/// `appearance.fold_sections`: whether a note opens with its sections
/// folded, as the vault says, always, or never.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FoldSections {
    #[default]
    Vault,
    Always,
    Never,
}

impl FoldSections {
    pub const ALL: [FoldSections; 3] = [FoldSections::Vault, FoldSections::Always, FoldSections::Never];

    /// The setting as written; anything else is the vault's word.
    pub fn named(name: &str) -> FoldSections {
        match name.trim() {
            "always" => FoldSections::Always,
            "never" => FoldSections::Never,
            _ => FoldSections::Vault,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FoldSections::Vault => "vault",
            FoldSections::Always => "always",
            FoldSections::Never => "never",
        }
    }

    /// Whether notes open folded, with what the vault says.
    pub fn folds(self, vault: VaultSettings) -> bool {
        match self {
            FoldSections::Vault => vault.folded,
            FoldSections::Always => true,
            FoldSections::Never => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_vault_says_whether_notes_open_folded() {
        assert!(!VaultSettings::default().folded);
        assert!(!VaultSettings::new("").folded);
        assert!(VaultSettings::new("[reader]\nfolded = true\n").folded);
        assert!(!VaultSettings::new("[reader]\nfolded = false\n").folded);
        // Beside daftar-cli's own keys and sections, which are left alone.
        let text = "memory = true\n\n[audit]\nactive = [\"applied\", \"screening\"]\nquiet_days = 30\n\n[reader]\n# Notes open as their own table of contents.\nfolded = true\n";
        assert!(VaultSettings::new(text).folded);
        // The key of another section is not this one.
        assert!(!VaultSettings::new("[audit]\nfolded = true\n").folded);
    }

    #[test]
    fn a_vault_with_no_file_has_the_default() {
        let root = std::env::temp_dir().join(format!("daftar-vault-settings-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(VaultSettings::read(&root), VaultSettings::default());
        std::fs::write(root.join(".daftar.toml"), "[reader]\nfolded = true\n").unwrap();
        assert!(VaultSettings::read(&root).folded);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_app_follows_the_vault_or_says_otherwise() {
        let folded = VaultSettings { folded: true };
        let open = VaultSettings::default();
        assert_eq!(FoldSections::named("vault"), FoldSections::Vault);
        assert_eq!(FoldSections::named("always"), FoldSections::Always);
        assert_eq!(FoldSections::named("never"), FoldSections::Never);
        // Absent or unknown is the vault's word.
        assert_eq!(FoldSections::named(""), FoldSections::Vault);
        assert_eq!(FoldSections::named("sometimes"), FoldSections::Vault);
        assert!(FoldSections::Vault.folds(folded) && !FoldSections::Vault.folds(open));
        assert!(FoldSections::Always.folds(open));
        assert!(!FoldSections::Never.folds(folded));
        for choice in FoldSections::ALL {
            assert_eq!(FoldSections::named(choice.name()), choice);
        }
    }
}
