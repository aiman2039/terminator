//! Changed framebuffer rectangles since the last shared-memory publish.
use terminator_core::vnc::{MAX_SHM_REGIONS, VncRegion};

/// `Full` means every pixel may have changed (new size, new viewer).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum DirtyRects {
    #[default]
    Full,
    Rects(Vec<VncRegion>),
}

impl DirtyRects {
    /// Nothing changed yet.
    pub(super) fn clean() -> Self {
        Self::Rects(Vec::new())
    }

    /// Add `region`. Rectangles inside another are dropped; past
    /// [`MAX_SHM_REGIONS`] the list collapses into its bounding rectangle,
    /// so a busy screen costs one copy instead of many small ones.
    pub(super) fn add(&mut self, region: VncRegion) {
        let Self::Rects(rects) = self else {
            return;
        };
        if region.is_empty() || rects.iter().any(|r| contains(*r, region)) {
            return;
        }
        rects.retain(|r| !contains(region, *r));
        rects.push(region);
        if rects.len() > MAX_SHM_REGIONS {
            let union = rects
                .iter()
                .fold(VncRegion::default(), |acc, r| acc.union(*r));
            *rects = vec![union];
        }
    }

    /// The rectangles to copy; empty means the whole frame.
    pub(super) fn take(&mut self) -> Vec<VncRegion> {
        match std::mem::replace(self, Self::clean()) {
            Self::Full => Vec::new(),
            Self::Rects(rects) => rects,
        }
    }
}

fn contains(outer: VncRegion, inner: VncRegion) -> bool {
    let right = |r: VncRegion| u32::from(r.x).saturating_add(u32::from(r.width));
    let bottom = |r: VncRegion| u32::from(r.y).saturating_add(u32::from(r.height));
    outer.x <= inner.x
        && outer.y <= inner.y
        && right(outer) >= right(inner)
        && bottom(outer) >= bottom(inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, width: u16, height: u16) -> VncRegion {
        VncRegion {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn separate_changes_stay_separate_rectangles() {
        let mut dirty = DirtyRects::clean();
        dirty.add(rect(0, 0, 10, 10));
        dirty.add(rect(1000, 700, 20, 20));
        dirty.add(rect(2, 2, 3, 3));
        assert_eq!(
            dirty.take(),
            vec![rect(0, 0, 10, 10), rect(1000, 700, 20, 20)]
        );
        assert_eq!(dirty.take(), Vec::new(), "take leaves it clean");
        assert_eq!(dirty, DirtyRects::clean());
    }

    #[test]
    fn too_many_rectangles_collapse_into_their_bounds() {
        let mut dirty = DirtyRects::clean();
        for i in 0..=u16::try_from(MAX_SHM_REGIONS).unwrap() {
            dirty.add(rect(i * 4, 0, 2, 2));
        }
        let bounds = u16::try_from(MAX_SHM_REGIONS).unwrap() * 4 + 2;
        assert_eq!(dirty.take(), vec![rect(0, 0, bounds, 2)]);
    }

    #[test]
    fn a_full_frame_ignores_rectangles_until_taken() {
        let mut dirty = DirtyRects::Full;
        dirty.add(rect(0, 0, 1, 1));
        assert!(dirty.take().is_empty(), "empty means the whole frame");
        dirty.add(rect(0, 0, 1, 1));
        assert_eq!(dirty.take(), vec![rect(0, 0, 1, 1)]);
    }
}
