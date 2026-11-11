//! Who a Task's outward write is on behalf of, beside the identity it acts as.

/// The people a Task's outward write credits, read off its Task and Sandbox.
///
/// The [`moatless_types::access_control::Actor`] decides the credential a write
/// presents; this decides who the write names as the human behind it, on
/// every surface: a commit's `Co-authored-by`, a pull request's assignee, a
/// Linear issue's reporter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attribution {
    /// The person whose message switched the Sandbox to them. `None` while its
    /// owner, or only a Loop, has prompted it.
    pub driver: Option<String>,
    /// The Task's Reporter.
    pub reporter: Option<String>,
    /// The Task's owner.
    pub owner: Option<String>,
}

impl Attribution {
    /// Who a write `caller` makes for a Task with `owner` and `reporter`
    /// credits; with no Task, `owner` is `None` and the caller is credited.
    ///
    /// A pod authenticates as its Sandbox's driver, else its owner, so a
    /// `caller` other than the owner is the driver.
    pub fn of_caller(caller: &str, owner: Option<&str>, reporter: Option<&str>) -> Self {
        let owner = owner.unwrap_or(caller);
        Self {
            driver: (caller != owner).then(|| caller.to_string()),
            reporter: reporter.map(str::to_string),
            owner: Some(owner.to_string()),
        }
    }

    /// The people to credit, most specific first and each once: the driver,
    /// then the Reporter, then the owner.
    ///
    /// The Reporter outranks the owner because a Loop's Task is owned by the
    /// Loop's user or bot, who did not ask for the work.
    pub fn credited(&self) -> impl Iterator<Item = &str> {
        let mut seen: Vec<&str> = Vec::with_capacity(3);
        [&self.driver, &self.reporter, &self.owner]
            .into_iter()
            .filter_map(|person| person.as_deref())
            .filter(|person| !person.is_empty())
            .filter(move |person| {
                if seen.contains(person) {
                    return false;
                }
                seen.push(person);
                true
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn attribution(
        driver: Option<&str>,
        reporter: Option<&str>,
        owner: Option<&str>,
    ) -> Attribution {
        Attribution {
            driver: driver.map(str::to_string),
            reporter: reporter.map(str::to_string),
            owner: owner.map(str::to_string),
        }
    }

    #[test]
    fn the_driver_is_credited_before_the_reporter_and_the_owner() {
        let all = attribution(Some("ada"), Some("grace"), Some("loop-bot"));
        assert_eq!(
            all.credited().collect::<Vec<_>>(),
            ["ada", "grace", "loop-bot"]
        );
    }

    #[test]
    fn a_loop_task_nobody_drove_credits_its_reporter_before_its_owner() {
        let undriven = attribution(None, Some("grace"), Some("loop-bot"));
        assert_eq!(undriven.credited().next(), Some("grace"));
    }

    #[test]
    fn a_caller_other_than_the_owner_is_the_driver() {
        assert_eq!(
            Attribution::of_caller("ada", Some("loop-bot"), Some("grace")),
            attribution(Some("ada"), Some("grace"), Some("loop-bot"))
        );
        assert_eq!(
            Attribution::of_caller("loop-bot", Some("loop-bot"), None).driver,
            None,
            "the owner's own pod drives nobody else's work"
        );
        assert_eq!(
            Attribution::of_caller("ada", None, None).owner.as_deref(),
            Some("ada"),
            "a write naming no Task credits its caller"
        );
    }

    #[test]
    fn each_person_is_credited_once() {
        let same = attribution(Some("ada"), Some(""), Some("ada"));
        assert_eq!(same.credited().collect::<Vec<_>>(), ["ada"]);
    }
}
