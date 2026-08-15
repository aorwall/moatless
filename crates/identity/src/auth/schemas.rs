//! Auth API request/response schemas.
//!
//! Re-exported from the `moatless-api-schemas` crate.

pub use moatless_api_schemas::auth::*;

use super::models::UserRole;

pub fn user_role_from_wire(role: moatless_api_schemas::enums::UserRole) -> UserRole {
    match role {
        moatless_api_schemas::enums::UserRole::User => UserRole::User,
        moatless_api_schemas::enums::UserRole::Admin => UserRole::Admin,
    }
}

pub fn user_role_to_wire(role: UserRole) -> moatless_api_schemas::enums::UserRole {
    match role {
        UserRole::User => moatless_api_schemas::enums::UserRole::User,
        UserRole::Admin => moatless_api_schemas::enums::UserRole::Admin,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_user_role_maps_both_ways_with_identical_json() {
        for domain in [UserRole::User, UserRole::Admin] {
            let wire = user_role_to_wire(domain);
            assert_eq!(user_role_from_wire(wire), domain);
            assert_eq!(
                serde_json::to_value(domain).expect("serialize"),
                serde_json::to_value(wire).expect("serialize")
            );
        }
    }
}
