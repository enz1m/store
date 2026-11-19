//! Store routes.

// ---------------------------------------------------------------------------
// Routes
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Route {
    #[default]
    Discover,
    Categories,
    Library,
    Search,
    Detail,
}

impl Route {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Route::Discover => "discover",
            Route::Categories => "categories",
            Route::Library => "library",
            Route::Search => "search",
            Route::Detail => "detail",
        }
    }
    pub(crate) fn title(self) -> &'static str {
        match self {
            Route::Discover => crate::i18n::label("Discover"),
            Route::Categories => crate::i18n::label("Categories"),
            Route::Library => crate::i18n::label("Library"),
            Route::Search => crate::i18n::label("Search"),
            Route::Detail => crate::i18n::label("Details"),
        }
    }
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Route::Discover => "compass",
            Route::Categories => "layout-grid",
            Route::Library => "library",
            Route::Search => "search",
            Route::Detail => "info",
        }
    }
    /// A pushed page shows a back affordance in the top bar.
    pub(crate) fn is_detail(self) -> bool {
        matches!(self, Route::Detail)
    }
}
