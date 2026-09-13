/// A closed set of values persisted as text and carried on the wire as text.
///
/// Generates the wire mapping (`as_str`, `Display`, `TryFrom<&str>`,
/// `TryFrom<String>`) and nothing else. Reading one out of a database column
/// belongs to the crate that owns the query, not here.
///
/// Keep this invocation name (`string_enum!`) stable: architecture analysis
/// greps for it to find every domain enum.
///
/// Two knobs beyond the bare form:
/// - `Name [Hash, Default] { ... }` adds derives beyond the default set.
/// - `Name using "lowercase" { ... }` overrides the default `snake_case`
///   `#[serde(rename_all = ...)]`.
///
/// A variant's `$value` is what `as_str`/`Display`/serde use; `$value | alias`
/// accepts additional strings (e.g. a retired spelling from old rows) in
/// `TryFrom` only.
macro_rules! string_enum {
    (
        $(#[$meta:meta])*
        $name:ident [$($extra:ident),+ $(,)?] using $rename:literal {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [$($extra),+] $rename {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        $(#[$meta:meta])*
        $name:ident [$($extra:ident),+ $(,)?] {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [$($extra),+] "snake_case" {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        $(#[$meta:meta])*
        $name:ident using $rename:literal {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [] $rename {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        $(#[$meta:meta])*
        $name:ident { $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)? }
    ) => {
        $crate::string_enum!(@impl
            $(#[$meta])* $name [] "snake_case" {
                $($(#[$vmeta])* $variant => $value $(| $alias)*),+
            }
        );
    };
    (
        @impl
        $(#[$meta:meta])*
        $name:ident [$($extra:ident),*] $rename:literal {
            $($(#[$vmeta:meta])* $variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, $($extra,)* serde::Serialize, serde::Deserialize)]
        #[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
        #[serde(rename_all = $rename)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
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
