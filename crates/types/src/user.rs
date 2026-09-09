//! User vocabulary: what a User may do.

crate::string_enum!(
    /// Role assigned to a user.
    UserRole {
        User => "user",
        Admin => "admin",
    }
);

crate::string_enum!(
    /// Whether a user may sign in and be routed work.
    UserStatus {
        Active => "active",
        /// A departed person, or a superseded duplicate account. Every login
        /// path refuses one, and the directory hides one by default.
        Disabled => "disabled",
    }
);
