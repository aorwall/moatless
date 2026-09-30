/// A closed set of values persisted as text and carried on the wire as text.
///
/// Declares the enum, then delegates to [`string_enum_mapping!`] for the wire
/// mapping. Reading one out of a database column belongs to the crate that
/// owns the query, not here.
///
/// Keep this invocation name (`string_enum!`) stable: architecture analysis
/// greps for it to find every domain enum.
///
/// One knob beyond the bare form: `Name [Hash, Default] { ... }` adds derives
/// beyond the default set. A type that must not carry this macro's serde
/// derive and wire contract — e.g. one with no wire representation, or one
/// that keeps its own smaller derive set — declares its own enum and invokes
/// [`string_enum_mapping!`] directly instead of this macro.
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
            $(
                $(#[$vmeta])*
                #[serde(rename = $value)]
                $(#[serde(alias = $alias)])*
                // utoipa reads no `serde(rename)` off a unit variant, so without
                // this the dumped schema declares the Rust identifier and the
                // client generated from it never matches a real response.
                #[cfg_attr(feature = "openapi", schema(rename = $value))]
                $variant
            ),+
        }

        $crate::string_enum_mapping!(
            $name {
                $($variant => $value $(| $alias)*),+
            }
        );
    };
}

/// Generates `as_str`, `Display`, `FromStr`, `TryFrom`, and `const ALL` over
/// an enum the caller already declared — for a type that wants its own derive
/// list rather than [`string_enum!`]'s. Unlike that macro, the caller declares
/// the enum, so architecture analysis indexes it directly and this name is not
/// part of the grep contract.
macro_rules! string_enum_mapping {
    (
        $name:ident {
            $($variant:ident => $value:literal $(| $alias:literal)*),+ $(,)?
        }
    ) => {
        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            /// Every value, so a caller can assert a closed mapping names all of
            /// them.
            pub const ALL: &'static [Self] = &[
                $(Self::$variant),+
            ];
        }

        impl std::fmt::Display for $name {
            // Matches on `self` directly rather than through `as_str(self)`:
            // `as_str` takes `self` by value, which would move `*self` out
            // from behind the `&self` a non-`Copy` caller's enum gets here.
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(match self {
                    $(Self::$variant => $value,)+
                })
            }
        }

        impl std::str::FromStr for $name {
            type Err = String;

            fn from_str(value: &str) -> Result<Self, String> {
                Self::try_from(value)
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
pub(crate) use string_enum_mapping;
