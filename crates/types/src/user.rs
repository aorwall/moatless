//! User vocabulary: what a User may do.

crate::string_enum!(
    /// Role assigned to a user.
    UserRole {
        User => "user",
        Admin => "admin",
    }
);
