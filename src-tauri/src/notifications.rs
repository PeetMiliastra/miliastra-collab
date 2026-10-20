//! Only the creator-approved popup events. Operational status stays in the app.
pub enum Notice<'a> {
    Reserved(&'a str),
    ReservedBy {
        creator: &'a str,
        slice: &'a str,
    },
    FinishRequested {
        creator: &'a str,
        slice: &'a str,
    },
    TakenOver {
        creator: &'a str,
        slice: &'a str,
    },
    UnsharedChanges(&'a str),
    AutoReleased(&'a str),
    Downloaded {
        creator: &'a str,
        count: usize,
    },
    Applied {
        creator: Option<&'a str>,
        slices: &'a str,
    },
}
impl Notice<'_> {
    pub fn message(&self) -> String {
        match self {
            Self::Reserved(slice) => format!("Reserved {slice}"),
            Self::ReservedBy { creator, slice } => format!("{creator} is already working on {slice}! Ask them to finish before you edit this."),
            Self::FinishRequested { creator, slice } => format!("{creator} is asking you to finish work on {slice}"),
            Self::TakenOver { creator, slice } => format!("{creator} took over {slice}! If you both change the slice, this will create conflicts."),
            Self::UnsharedChanges(slice) => format!("You still have unshared changes to {slice}"),
            Self::AutoReleased(slices) => format!("Auto-released {slices} because of inactivity"),
            Self::Downloaded { creator, count } => format!("{creator} shared update: {count} slice updates downloaded."),
            Self::Applied { creator: Some(creator), slices } => format!("Applied {creator}’s update to {slices}."),
            Self::Applied { creator: None, slices } => format!("Applied teammates’ updates to {slices}."),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn popup_copy_matches_the_creators_replacements() {
        for (notice, expected) in [
            (Notice::Reserved("Boats"), "Reserved Boats"),
            (
                Notice::ReservedBy {
                    creator: "Alex",
                    slice: "Boats",
                },
                "Alex is already working on Boats! Ask them to finish before you edit this.",
            ),
            (
                Notice::FinishRequested {
                    creator: "Alex",
                    slice: "Boats",
                },
                "Alex is asking you to finish work on Boats",
            ),
            (
                Notice::TakenOver {
                    creator: "Alex",
                    slice: "Boats",
                },
                "Alex took over Boats! If you both change the slice, this will create conflicts.",
            ),
            (
                Notice::UnsharedChanges("Boats"),
                "You still have unshared changes to Boats",
            ),
            (
                Notice::AutoReleased("Boats, Islands"),
                "Auto-released Boats, Islands because of inactivity",
            ),
            (
                Notice::Downloaded {
                    creator: "Alex",
                    count: 1,
                },
                "Alex shared update: 1 slice updates downloaded.",
            ),
            (
                Notice::Downloaded {
                    creator: "Alex",
                    count: 3,
                },
                "Alex shared update: 3 slice updates downloaded.",
            ),
            (
                Notice::Applied {
                    creator: Some("Alex"),
                    slices: "Boats",
                },
                "Applied Alex’s update to Boats.",
            ),
            (
                Notice::Applied {
                    creator: None,
                    slices: "Boats, Islands",
                },
                "Applied teammates’ updates to Boats, Islands.",
            ),
        ] {
            assert_eq!(notice.message(), expected);
        }
    }
    #[test]
    fn creator_and_slice_names_are_preserved_verbatim() {
        let slice = format!("Harbor {} 島 {{1}}", char::from(183u8));
        let notice = Notice::ReservedBy {
            creator: "Zoë",
            slice: &slice,
        }
        .message();
        assert_eq!(
            notice,
            format!("Zoë is already working on {slice}! Ask them to finish before you edit this.")
        );
    }
}
