//! Setting an Artifact's visibility, against a mocked setter.

use super::fixtures::*;
use super::*;
use crate::ports::{ArtifactVisibilityError, MockArtifactVisibilitySetter};
use moatless_types::task::Visibility;

fn state_with(setter: MockArtifactVisibilitySetter) -> UiRpcState {
    UiRpcState {
        artifacts: Arc::new(setter),
        ..state()
    }
}

async fn set_visibility(setter: MockArtifactVisibilitySetter, visibility: &str) -> Value {
    let frame = request(
        "1",
        "artifacts.setVisibility",
        json!({ "artifactId": "art-1", "visibility": visibility }),
    );
    let mut session = session();
    let output = session
        .handle_json(&frame, &state_with(setter), &viewer())
        .await;
    output.frames[0]["exit"].clone()
}

#[tokio::test]
async fn set_visibility_answers_the_visibility_it_set() {
    let mut setter = MockArtifactVisibilitySetter::new();
    setter
        .expect_set_visibility()
        .withf(|_, id, visibility| id == "art-1" && *visibility == Visibility::Private)
        .times(1)
        .returning(|_, _, _| Ok(()));

    let exit = set_visibility(setter, "private").await;

    assert_eq!(exit["_tag"], "Success", "{exit}");
    assert_eq!(
        exit["value"],
        json!({ "artifactId": "art-1", "visibility": "private" })
    );
}

/// A refusal is the contract's declared error, so the panel can show why
/// rather than a generic failure.
#[tokio::test]
async fn a_viewer_who_cannot_change_the_task_is_refused_in_the_contracts_terms() {
    for (refusal, message) in [
        (ArtifactVisibilityError::Forbidden, "Only someone"),
        (ArtifactVisibilityError::NotFound, "not available"),
    ] {
        let mut setter = MockArtifactVisibilitySetter::new();
        let mut refusal = Some(refusal);
        setter
            .expect_set_visibility()
            .returning(move |_, _, _| Err(refusal.take().expect("called once")));

        let exit = set_visibility(setter, "public").await;

        let cause = &exit["cause"][0];
        assert_eq!(cause["_tag"], "Fail", "{exit}");
        assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
        assert_eq!(cause["error"]["requiredScope"], "orchestration:operate");
        assert!(
            cause["error"]["message"]
                .as_str()
                .is_some_and(|text| text.contains(message)),
            "{exit}"
        );
    }
}

/// The wire's visibility is the contract's two literals; anything else is a
/// malformed request, refused before the setter is reached.
#[tokio::test]
async fn an_unknown_visibility_is_refused_before_the_setter() {
    let mut setter = MockArtifactVisibilitySetter::new();
    setter.expect_set_visibility().never();

    let exit = set_visibility(setter, "team").await;

    assert_eq!(exit["cause"][0]["_tag"], "Die", "{exit}");
}
