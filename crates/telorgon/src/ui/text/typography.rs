/// Runtime-local generic font selection, shared by GUI, shell and embedded text.
/// Explicit family names on individual controls remain unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Typography {
    pub sans_serif: String,
    pub serif: String,
    pub monospace: String,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            sans_serif: if cfg!(feature = "font-inter") {
                "Inter"
            } else {
                "sans-serif"
            }
            .into(),
            serif: if cfg!(feature = "font-source-serif-4") {
                "Source Serif 4"
            } else {
                "serif"
            }
            .into(),
            monospace: if cfg!(feature = "font-jetbrains-mono") {
                "JetBrains Mono"
            } else {
                "monospace"
            }
            .into(),
        }
    }
}

impl Typography {
    pub fn ui_font(mut self, family: impl Into<String>) -> Self {
        self.sans_serif = family.into();
        self
    }

    pub(crate) fn family<'a>(&'a self, family: &'a str) -> &'a str {
        match family {
            "sans-serif" | "sans_serif" => &self.sans_serif,
            "serif" => &self.serif,
            "monospace" => &self.monospace,
            _ => family,
        }
    }
}
