use std::ops::ControlFlow;

use serde::Serialize;
use thiserror::Error;

use crate::geometry::Rect;

pub use crate::platform::macos::ax::{element_at, elements_in_window};

pub const DEFAULT_MAX_ELEMENTS: usize = 200;
pub const MAX_ELEMENTS: usize = 1_000;
const MAX_DEPTH: usize = 32;
const MAX_VISITED: usize = 5_000;

#[derive(Debug, Error)]
pub enum ElementError {
    #[error("Accessibility permission is required")]
    AccessibilityDenied,
    #[error("The screen is locked")]
    ScreenLocked,
    #[error("No accessibility element with usable bounds was found")]
    ElementNotFound,
    #[error("Window {0} was not found")]
    WindowNotFound(u32),
    #[error("Accessibility did not respond to {operation} (AX error -25204)")]
    Timeout { operation: &'static str },
    #[error("Accessibility call {operation} failed (AX error {code})")]
    Ax { operation: &'static str, code: i32 },
}

#[derive(Debug, Serialize)]
pub struct ElementInfo {
    pub role: String,
    pub subrole: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
    pub bounds: Rect<f64>,
    pub enabled: Option<bool>,
    pub focused: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ElementAncestor {
    pub role: Option<String>,
    pub bounds: Option<Rect<f64>>,
}

#[derive(Debug)]
pub struct ElementHit {
    pub element: ElementInfo,
    pub ancestors: Vec<ElementAncestor>,
}

pub struct ElementFilter<'a> {
    pub roles: &'a [String],
    pub max_results: usize,
}

impl Default for ElementFilter<'_> {
    fn default() -> Self {
        Self {
            roles: &[],
            max_results: DEFAULT_MAX_ELEMENTS,
        }
    }
}

impl ElementFilter<'_> {
    pub(crate) fn matches(&self, role: &str, bounds: Rect<f64>, window: Rect<f64>) -> bool {
        (self.roles.is_empty() || self.roles.iter().any(|candidate| candidate == role))
            && bounds.intersection(window).is_some()
    }
}

#[derive(Debug)]
pub struct ListedElement {
    pub id: usize,
    pub element: ElementInfo,
}

#[derive(Debug, Default)]
pub struct ElementList {
    pub elements: Vec<ListedElement>,
    pub truncated: bool,
}

pub(crate) fn walk<N, I: Iterator<Item = N>>(
    root: &N,
    max_results: usize,
    inspect: impl Fn(&N) -> Result<Option<ElementInfo>, ElementError>,
    children: impl Fn(&N, usize) -> Result<I, ElementError>,
) -> Result<ElementList, ElementError> {
    let mut traversal = Traversal {
        result: ElementList::default(),
        visited: 0,
        max_results: max_results.min(MAX_ELEMENTS),
    };
    let _ = traversal.visit(root, 0, &inspect, &children)?;
    Ok(traversal.result)
}

struct Traversal {
    result: ElementList,
    visited: usize,
    max_results: usize,
}

impl Traversal {
    fn visit<N, I: Iterator<Item = N>>(
        &mut self,
        node: &N,
        depth: usize,
        inspect: &impl Fn(&N) -> Result<Option<ElementInfo>, ElementError>,
        children: &impl Fn(&N, usize) -> Result<I, ElementError>,
    ) -> Result<ControlFlow<()>, ElementError> {
        if self.visited == MAX_VISITED {
            self.result.truncated = true;
            return Ok(ControlFlow::Break(()));
        }
        self.visited += 1;
        if let Some(element) = inspect(node)? {
            if self.result.elements.len() == self.max_results {
                self.result.truncated = true;
                return Ok(ControlFlow::Break(()));
            }
            self.result.elements.push(ListedElement {
                id: self.visited,
                element,
            });
        }
        if depth == MAX_DEPTH {
            self.result.truncated |= children(node, 1)?.next().is_some();
        } else {
            for child in children(node, MAX_VISITED - self.visited + 1)? {
                if self.visit(&child, depth + 1, inspect, children)?.is_break() {
                    return Ok(ControlFlow::Break(()));
                }
            }
        }
        Ok(ControlFlow::Continue(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: Rect<f64> = Rect {
        origin: crate::geometry::Point { x: -10., y: 0. },
        size: crate::geometry::Size {
            width: 20.,
            height: 20.,
        },
    };

    fn button() -> ElementInfo {
        ElementInfo {
            role: "AXButton".into(),
            subrole: None,
            title: None,
            description: None,
            value: None,
            identifier: None,
            bounds: Rect::new(0., 0., 1., 1.),
            enabled: None,
            focused: None,
        }
    }

    #[test]
    fn filters_by_exact_role_and_positive_window_overlap() {
        let roles = ["AXButton".into(), "AXLink".into()];
        let filter = ElementFilter {
            roles: &roles,
            ..ElementFilter::default()
        };
        assert!(filter.matches("AXLink", Rect::new(-15., 0., 6., 5.), WINDOW));
        assert!(!filter.matches("AXLink", Rect::new(-15., 0., 5., 5.), WINDOW));
        assert!(!filter.matches("axbutton", WINDOW, WINDOW));
        assert!(!filter.matches("AXTextField", WINDOW, WINDOW));
        assert!(ElementFilter::default().matches("AXTextField", WINDOW, WINDOW));
    }

    #[test]
    fn keeps_preorder_ids_when_filtering_and_marks_only_omitted_results() {
        let children = |node: &usize, _| {
            Ok(match node {
                0 => vec![1, 3, 4],
                1 => vec![2],
                _ => vec![],
            }
            .into_iter())
        };
        let inspect = |node: &usize| Ok(node.is_multiple_of(2).then(button));
        let exact = walk(&0, 3, inspect, children).unwrap();
        assert_eq!(
            exact
                .elements
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [1, 3, 5]
        );
        assert!(!exact.truncated);
        let limited = walk(&0, 2, inspect, children).unwrap();
        assert_eq!(
            limited
                .elements
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            [1, 3]
        );
        assert!(limited.truncated);
    }

    #[test]
    fn caps_depth_and_visits_even_without_matches() {
        let deep = walk(&0, 1, |_| Ok(None), |node, _| Ok(std::iter::once(node + 1))).unwrap();
        assert!(deep.elements.is_empty());
        assert!(deep.truncated);
        let wide = walk(
            &0,
            1,
            |_| Ok(None),
            |node, limit| Ok(1..if *node == 0 { limit + 1 } else { 1 }),
        )
        .unwrap();
        assert!(wide.elements.is_empty());
        assert!(wide.truncated);
    }
}
