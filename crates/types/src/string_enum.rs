/// A closed set of values persisted as text and carried on the wire as text.
///
/// Generates the wire mapping (`as_str`, `Display`, `TryFrom<&str>`,
/// `TryFrom<String>`) and nothing else. Reading one out of a database column
/// belongs to the crate that owns the query, not here.
///
/// Keep this invocation name (`string_enum!`) stable: architecture analysis
/// greps for it to find every domain enum.
///
/// One knob beyond the bare form: `Name [Hash, Default] { ... }` adds derives
/// beyond the default set.
///
/// A variant's `$value` is what `as_str`/`Display`/serde use — each variant
/// gets its own `#[serde(rename = $value)]`, so there is no `rename_all` to
/// drift out of step with it. `$value | alias` accepts additional strings
/// (e.g. a retired spelling from old rows) in `TryFrom` and `Deserialize`
/// both, via `#[serde(alias = ...)]`.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident [$($extra:ident),+ $(,)?] {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [$($extra),+] {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)? }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [] {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        @impl
        $(#[$meta:meta])*
        $name:ident [$($extra:ident),*] {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, $($extra,)* serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        pub enum $name {
            $($(#[$vmeta])* #[serde(rename = $value)] $(#[serde(alias = $alias)])* $variant),+
        }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl TryFrom<&str> for $name {
            type Error = String;

            fn try_from(value: &str) -> Result<Self, String> {
                match value {
                    $($value $(| $alias)* => Ok(Self::$variant)),+,
                    other => Err(format!("unknown {}: {other:?}", stringify!($name))),
                }
            }
        }

        impl TryFrom<String> for $name {
            type Error = String;

            fn try_from(value: String) -> Result<Self, String> {
                Self::try_from(value.as_str())
            }
        }
    };
}

pub(crate) use string_enum;
