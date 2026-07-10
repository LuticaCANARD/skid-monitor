use crate::config;

#[derive(Clone, Copy)]
pub(crate) enum LayoutMode {
    Compact,
    Stacked,
    Split,
}

impl LayoutMode {
    pub(crate) fn for_width(width: f32) -> Self {
        if width < config::COMPACT_BREAKPOINT {
            Self::Compact
        } else if width < config::SPLIT_BREAKPOINT {
            Self::Stacked
        } else {
            Self::Split
        }
    }

    pub(crate) fn is_compact(self) -> bool {
        matches!(self, Self::Compact)
    }
}
