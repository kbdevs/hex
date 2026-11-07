//! A browser's open page, read from its Accessibility tree when its
//! scripting dictionary can't be asked (Automation off). The walk itself is
//! here, over any tree, so it is tested on every OS; `accessibility` feeds it
//! the live one on macOS.
//!
//! Chromium browsers behind another app often have no focused element and
//! no focused window, and Arc's native window chrome (sidebar, toolbar)
//! sits between the window and the page. Read one selected window, breadth
//! first and bounded. Keep the URL and title from the same web area. With no
//! web area, its window title can still name a call (#827, #831).

use std::collections::VecDeque;

/// One element of an Accessibility tree, as the walk reads it.
pub(crate) trait AxNode: Sized {
    fn role(&self) -> Option<String>;
    fn title(&self) -> Option<String>;
    /// The page address of a web area (`AXURL`).
    fn url(&self) -> Option<String>;
    fn children(&self) -> Vec<Self>;
}

/// One page's address and title. An ambiguous window has neither.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct BrowserPage {
    pub(crate) url: Option<String>,
    pub(crate) title: Option<String>,
}

fn nonempty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// How many elements one window's search reads at most. Arc's chrome is a
/// few hundred; a page's own content is never entered.
const MAX_NODES_PER_WINDOW: usize = 400;
const MAX_DEPTH: usize = 40;

/// Reads only the first selected window. Other windows and web-area contents
/// are not searched. More than one page, or an incomplete walk, stays unknown.
pub(crate) fn page<N: AxNode>(windows: impl IntoIterator<Item = N>) -> BrowserPage {
    let Some(window) = windows.into_iter().next() else {
        return BrowserPage::default();
    };
    let window_title = nonempty(window.title());
    let mut found = None;
    let mut queue = VecDeque::from([(window, 0)]);
    let mut read = 0;
    while let Some((node, depth)) = queue.pop_front() {
        read += 1;
        if read > MAX_NODES_PER_WINDOW {
            return BrowserPage::default();
        }
        if node.role().as_deref() == Some("AXWebArea") {
            // ponytail: split views stay unknown; add window-scoped focus
            // resolution if naming calls in split views becomes necessary.
            if found.is_some() {
                return BrowserPage::default();
            }
            found = Some(BrowserPage {
                url: nonempty(node.url()),
                title: nonempty(node.title()),
            });
            continue;
        }
        if depth >= MAX_DEPTH {
            return BrowserPage::default();
        }
        queue.extend(node.children().into_iter().map(|child| (child, depth + 1)));
    }
    found.unwrap_or(BrowserPage {
        url: None,
        title: window_title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A captured element: role, title, URL, children.
    #[derive(Clone)]
    struct Fixture(
        &'static str,
        Option<&'static str>,
        Option<&'static str>,
        Vec<Fixture>,
    );

    impl AxNode for Fixture {
        fn role(&self) -> Option<String> {
            Some(self.0.into())
        }
        fn title(&self) -> Option<String> {
            self.1.map(Into::into)
        }
        fn url(&self) -> Option<String> {
            self.2.map(Into::into)
        }
        fn children(&self) -> Vec<Self> {
            self.3.clone()
        }
    }

    fn element(role: &'static str, children: Vec<Fixture>) -> Fixture {
        Fixture(role, None, None, children)
    }

    /// Arc's shape: the page sits under the window's split view beside the
    /// sidebar, and the window title is the space, not the tab.
    fn arc_window() -> Fixture {
        Fixture(
            "AXWindow",
            Some("Work"),
            None,
            vec![element(
                "AXGroup",
                vec![element(
                    "AXSplitGroup",
                    vec![
                        element(
                            "AXGroup",
                            vec![
                                Fixture("AXTextField", Some("Search or Enter URL…"), None, vec![]),
                                Fixture("AXButton", Some("Meet - abc-defg-hij"), None, vec![]),
                            ],
                        ),
                        element(
                            "AXGroup",
                            vec![element(
                                "AXGroup",
                                vec![Fixture(
                                    "AXWebArea",
                                    Some("Meet - abc-defg-hij"),
                                    Some("https://meet.google.com/abc-defg-hij"),
                                    vec![element("AXGroup", vec![])],
                                )],
                            )],
                        ),
                    ],
                )],
            )],
        )
    }

    #[test]
    fn arcs_page_is_found_beside_its_sidebar() {
        let little_arc = Fixture(
            "AXWindow",
            Some("Docs"),
            None,
            vec![Fixture(
                "AXWebArea",
                Some("Docs"),
                Some("https://docs.example.com/"),
                vec![],
            )],
        );
        let page = page([arc_window(), little_arc]);
        assert_eq!(
            page.url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(page.title.as_deref(), Some("Meet - abc-defg-hij"));
    }

    #[test]
    fn a_meeting_in_another_window_does_not_name_the_current_page() {
        let voice_note = Fixture(
            "AXWindow",
            Some("Voice note"),
            None,
            vec![Fixture(
                "AXWebArea",
                Some("Voice note"),
                Some("https://notes.example.com/"),
                vec![],
            )],
        );
        let page = page([voice_note, arc_window()]);
        assert_eq!(
            page,
            BrowserPage {
                url: Some("https://notes.example.com/".into()),
                title: Some("Voice note".into()),
            }
        );
    }

    #[test]
    fn a_voice_note_and_meeting_exposed_in_one_window_stay_unknown() {
        let voice_note = Fixture(
            "AXWebArea",
            Some("Voice note"),
            Some("https://notes.example.com/"),
            vec![],
        );
        let meeting = Fixture(
            "AXWebArea",
            Some("Meet - Design sync"),
            Some("https://meet.google.com/abc-defg-hij"),
            vec![],
        );
        for children in [
            vec![voice_note.clone(), meeting.clone()],
            vec![meeting, voice_note],
        ] {
            let window = Fixture("AXWindow", Some("Meet - Design sync"), None, children);
            assert_eq!(page([window]), BrowserPage::default());
        }
    }

    #[test]
    fn a_pages_missing_title_is_not_borrowed_from_the_window() {
        let window = Fixture(
            "AXWindow",
            Some("Meet - Design sync"),
            None,
            vec![Fixture("AXWebArea", None, None, vec![])],
        );
        assert_eq!(page([window]), BrowserPage::default());
    }

    #[test]
    fn an_incomplete_window_search_stays_unknown() {
        let mut window = arc_window();
        window
            .3
            .extend((0..MAX_NODES_PER_WINDOW).map(|_| element("AXGroup", vec![])));
        assert_eq!(page([window]), BrowserPage::default());
    }

    #[test]
    fn a_window_without_its_web_tree_still_gives_its_title() {
        // Chromium behind another app, its web accessibility still off.
        let page = page([Fixture(
            "AXWindow",
            Some("Meet - abc-defg-hij"),
            None,
            vec![],
        )]);
        assert!(page.url.is_none());
        assert_eq!(page.title.as_deref(), Some("Meet - abc-defg-hij"));
    }

    #[test]
    fn nothing_found_is_nothing_chosen() {
        assert_eq!(page(Vec::<Fixture>::new()), BrowserPage::default());
    }
}
