//! Plot widget entry points and builders.
//!
//! A [`Plot`] owns axis configuration, view mode, and a set of series. All
//! series in a plot share the same axes and transforms.

use crate::axis::AxisConfig;
use crate::interaction::Pin;
use crate::series::Series;
use crate::style::Theme;
use crate::view::{Range, View, Viewport};

/// Main plot widget container.
///
/// A plot is backend-agnostic and focuses on data, view state, and styling.
/// Render backends (such as the GPUI backend) drive viewport refreshes and
/// interaction state.
#[derive(Debug, Clone)]
pub struct Plot {
    theme: Theme,
    x_axis: AxisConfig,
    y_axis: AxisConfig,
    view: View,
    viewport: Option<Viewport>,
    initial_viewport: Option<Viewport>,
    series: Vec<Series>,
    pins: Vec<Pin>,
}

impl Plot {
    /// Create a plot with default configuration.
    ///
    /// Equivalent to `PlotBuilder::default().build()`.
    pub fn new() -> Self {
        Self {
            theme: Theme::default(),
            x_axis: AxisConfig::default(),
            y_axis: AxisConfig::default(),
            view: View::default(),
            viewport: None,
            initial_viewport: None,
            series: Vec::new(),
            pins: Vec::new(),
        }
    }

    /// Start building a plot with custom configuration.
    pub fn builder() -> PlotBuilder {
        PlotBuilder::default()
    }

    /// Access the current theme.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    /// Set the plot theme.
    pub fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    /// Access the X axis configuration.
    pub fn x_axis(&self) -> &AxisConfig {
        &self.x_axis
    }

    /// Access the Y axis configuration.
    pub fn y_axis(&self) -> &AxisConfig {
        &self.y_axis
    }

    /// Access the active view mode.
    pub fn view(&self) -> View {
        self.view
    }

    /// Access the current viewport.
    ///
    /// The viewport is computed by [`Plot::refresh_viewport`].
    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    /// Access all series.
    pub fn series(&self) -> &[Series] {
        &self.series
    }

    /// Access all series mutably.
    ///
    /// Returning the backing vector allows callers to add, remove, and reorder
    /// series at runtime.
    pub fn series_mut(&mut self) -> &mut Vec<Series> {
        &mut self.series
    }

    /// Add a series to the plot.
    ///
    /// The plot stores a shared handle instead of taking unique ownership.
    /// Appends made through other shared handles are visible immediately.
    pub fn add_series(&mut self, series: &Series) {
        self.series.push(series.share());
    }

    /// Access the pinned points.
    pub fn pins(&self) -> &[Pin] {
        &self.pins
    }

    /// Access the pinned points mutably.
    pub fn pins_mut(&mut self) -> &mut Vec<Pin> {
        &mut self.pins
    }

    /// Compute bounds across all visible series.
    pub fn data_bounds(&self) -> Option<Viewport> {
        let mut x_range: Option<Range> = None;
        let mut y_range: Option<Range> = None;
        for series in &self.series {
            if !series.is_visible() {
                continue;
            }
            if let Some(bounds) = series.bounds() {
                x_range = Some(match x_range {
                    None => bounds.x,
                    Some(existing) => Range::union(existing, bounds.x)?,
                });
                y_range = Some(match y_range {
                    None => bounds.y,
                    Some(existing) => Range::union(existing, bounds.y)?,
                });
            }
        }
        match (x_range, y_range) {
            (Some(x), Some(y)) => Some(Viewport::new(x, y)),
            _ => None,
        }
    }

    /// Access the initial viewport, if one was configured.
    ///
    /// The initial viewport is used as a fallback when no data is available yet,
    /// ensuring the plot renders with a valid axis range from the start.
    pub fn initial_viewport(&self) -> Option<Viewport> {
        self.initial_viewport
    }

    /// Set the initial viewport.
    ///
    /// This provides a sensible axis range before any data has been added to
    /// the plot. Once data arrives the configured [`View`] mode takes over.
    pub fn set_initial_viewport(&mut self, viewport: Viewport) {
        self.initial_viewport = Some(viewport);
    }

    /// Enter manual view with the given viewport.
    pub fn set_manual_view(&mut self, viewport: Viewport) {
        self.view = View::Manual;
        self.viewport = Some(viewport);
    }

    /// Reset to automatic view.
    pub fn reset_view(&mut self) {
        self.view = View::default();
        self.viewport = None;
    }

    /// Refresh the viewport based on the current view mode and data.
    ///
    /// This updates the cached viewport and applies padding to avoid tight
    /// bounds during auto-fit. When no data is available the `initial_viewport`
    /// is returned (and stored) so the plot always renders with a valid range.
    pub fn refresh_viewport(&mut self, padding_frac: f64, min_padding: f64) -> Option<Viewport> {
        let Some(bounds) = self.data_bounds() else {
            // No data yet — fall back to initial_viewport so the plot is
            // renderable and transitions cleanly once data arrives.
            if let Some(iv) = self.initial_viewport {
                self.viewport = Some(iv);
                return Some(iv);
            }
            return None;
        };
        match self.view {
            View::AutoAll { auto_x, auto_y } => {
                let mut next = bounds;
                if let Some(current) = self.viewport {
                    if !auto_x {
                        next.x = current.x;
                    }
                    if !auto_y {
                        next.y = current.y;
                    }
                }
                self.viewport = Some(next.padded(padding_frac, min_padding));
            }
            View::Manual => {
                if self.viewport.is_none() {
                    self.viewport = Some(bounds);
                }
            }
            View::FollowLastN { points } => {
                self.viewport = self.follow_last(points, false).or(self.initial_viewport);
            }
            View::FollowLastNXY { points } => {
                self.viewport = self.follow_last(points, true).or(self.initial_viewport);
            }
        }
        self.viewport
    }

    fn follow_last(&self, points: usize, follow_y: bool) -> Option<Viewport> {
        let mut max_series: Option<&Series> = None;
        let mut max_point: Option<crate::geom::Point> = None;
        for series in &self.series {
            if !series.is_visible() {
                continue;
            }
            let last_point = series.with_store(|store| store.data().points().last().copied());
            if let Some(point) = last_point
                && max_point.is_none_or(|max| point.x > max.x)
            {
                max_point = Some(point);
                max_series = Some(series);
            }
        }

        let max_series = max_series?;
        let max_point = max_point?;
        let (len, start_point) = max_series.with_store(|store| {
            let data = store.data();
            let len = data.len();
            let start_index = len.saturating_sub(points);
            (len, data.point(start_index))
        });
        if len == 0 {
            return None;
        }
        let start_point = start_point?;
        // Ensure the x range always has a positive span so that ticks can be
        // generated and the transform is valid even when all points share the
        // same X value (e.g. a single-point series).  We use the requested
        // window width as the minimum span so the axis feels proportional; if
        // the window is 0 we fall back to 1.0.
        let min_x_span = if points > 1 { (points - 1) as f64 } else { 1.0 };
        let x_range = Range::new(start_point.x, max_point.x).with_min_span(min_x_span);

        let y_range = if follow_y {
            let mut y_range: Option<Range> = None;
            for series in &self.series {
                if !series.is_visible() {
                    continue;
                }
                series.with_store(|store| {
                    let series_data = store.data();
                    let index_range = series_data.range_by_x(x_range);
                    for index in index_range {
                        if let Some(point) = series_data.point(index) {
                            y_range = Some(match y_range {
                                None => Range::new(point.y, point.y),
                                Some(mut existing) => {
                                    existing.expand_to_include(point.y);
                                    existing
                                }
                            });
                        }
                    }
                });
            }
            y_range?
        } else if let Some(current) = self.viewport {
            current.y
        } else if let Some(iv) = self.initial_viewport {
            iv.y
        } else {
            self.data_bounds()?.y
        };

        Some(Viewport::new(x_range, y_range))
    }
}

impl Default for Plot {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for configuring a plot before construction.
///
/// The builder captures theme, axes, view mode, and any initial series.
#[derive(Debug, Default)]
pub struct PlotBuilder {
    theme: Theme,
    x_axis: AxisConfig,
    y_axis: AxisConfig,
    view: View,
    initial_viewport: Option<Viewport>,
    series: Vec<Series>,
}

impl PlotBuilder {
    /// Set the theme used by the plot.
    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Set the X axis configuration.
    pub fn x_axis(mut self, axis: AxisConfig) -> Self {
        self.x_axis = axis;
        self
    }

    /// Set the Y axis configuration.
    pub fn y_axis(mut self, axis: AxisConfig) -> Self {
        self.y_axis = axis;
        self
    }

    /// Set the initial view mode.
    pub fn view(mut self, view: View) -> Self {
        self.view = view;
        self
    }

    /// Set the initial viewport used before any data is available.
    ///
    /// This ensures the plot renders with a valid axis range from the very
    /// first frame, avoiding the collapsed-at-zero appearance that occurs
    /// when data is streamed in gradually.
    ///
    /// # Example
    ///
    /// ```rust
    /// use gpui_liveplot::{Plot, Range, Viewport};
    ///
    /// let plot = Plot::builder()
    ///     .initial_viewport(Viewport::new(Range::new(0.0, 100.0), Range::new(-1.0, 1.0)))
    ///     .build();
    /// ```
    pub fn initial_viewport(mut self, viewport: Viewport) -> Self {
        self.initial_viewport = Some(viewport);
        self
    }

    /// Add a series to the plot.
    ///
    /// The builder stores a shared handle to the given series.
    pub fn series(mut self, series: &Series) -> Self {
        self.series.push(series.share());
        self
    }

    /// Build the plot.
    pub fn build(self) -> Plot {
        Plot {
            theme: self.theme,
            x_axis: self.x_axis,
            y_axis: self.y_axis,
            view: self.view,
            viewport: None,
            initial_viewport: self.initial_viewport,
            series: self.series,
            pins: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::series::Series;

    #[test]
    fn add_series_uses_shared_data_stream() {
        let mut source = Series::line("shared");
        let _ = source.extend_y([1.0, 2.0]);

        let mut plot = Plot::new();
        plot.add_series(&source);

        let initial_bounds = plot.data_bounds().expect("plot bounds");
        assert_eq!(initial_bounds.y.min, 1.0);
        assert_eq!(initial_bounds.y.max, 2.0);

        let _ = source.push_y(3.0);
        let next_bounds = plot.data_bounds().expect("plot bounds");
        assert_eq!(next_bounds.y.max, 3.0);
    }

    #[test]
    fn initial_viewport_used_when_no_data() {
        let iv = Viewport::new(Range::new(0.0, 100.0), Range::new(-1.0, 1.0));
        let mut plot = Plot::builder().initial_viewport(iv).build();

        // With no series at all, refresh_viewport should return the initial_viewport.
        let vp = plot.refresh_viewport(0.05, 1e-6).expect("initial viewport");
        assert_eq!(vp.x.min, 0.0);
        assert_eq!(vp.x.max, 100.0);
        assert_eq!(vp.y.min, -1.0);
        assert_eq!(vp.y.max, 1.0);
        // And the internal viewport cache should be populated too.
        assert_eq!(plot.viewport(), Some(iv));
    }

    #[test]
    fn initial_viewport_used_when_series_is_empty() {
        let iv = Viewport::new(Range::new(0.0, 50.0), Range::new(-5.0, 5.0));
        let mut plot = Plot::builder().initial_viewport(iv).build();
        let series = Series::line("empty");
        plot.add_series(&series);

        // Series exists but has no points — initial_viewport should still be returned.
        let vp = plot.refresh_viewport(0.05, 1e-6).expect("initial viewport");
        assert_eq!(vp.x.min, 0.0);
        assert_eq!(vp.x.max, 50.0);
    }

    #[test]
    fn data_bounds_take_over_from_initial_viewport() {
        let iv = Viewport::new(Range::new(0.0, 100.0), Range::new(-1.0, 1.0));
        let mut source = Series::line("sensor");
        let mut plot = Plot::builder().initial_viewport(iv).build();
        plot.add_series(&source);

        // Before data: initial_viewport wins.
        let vp_before = plot.refresh_viewport(0.0, 0.0).expect("initial viewport");
        assert_eq!(vp_before.x.min, 0.0);
        assert_eq!(vp_before.x.max, 100.0);

        // Push some data points.
        let _ = source.extend_y([10.0, 20.0, 30.0]);

        // After data: AutoAll view should expand to fit the actual data.
        let vp_after = plot.refresh_viewport(0.0, 0.0).expect("data viewport");
        // X range for indexed data is 0..2 (indices).
        assert_eq!(vp_after.x.min, 0.0);
        assert_eq!(vp_after.x.max, 2.0);
        assert_eq!(vp_after.y.min, 10.0);
        assert_eq!(vp_after.y.max, 30.0);
    }

    #[test]
    fn no_initial_viewport_returns_none_without_data() {
        let mut plot = Plot::new();
        // No initial_viewport set, no series — must return None.
        assert!(plot.refresh_viewport(0.05, 1e-6).is_none());
    }

    #[test]
    fn follow_last_n_uses_initial_viewport_y_before_data() {
        let iv = Viewport::new(Range::new(0.0, 200.0), Range::new(-5.0, 5.0));
        let mut source = Series::line("sensor");
        let mut plot = Plot::builder()
            .view(View::FollowLastN { points: 200 })
            .initial_viewport(iv)
            .build();
        plot.add_series(&source);

        // No data yet — should fall back to initial_viewport entirely.
        let vp = plot.refresh_viewport(0.0, 0.0).expect("initial viewport");
        assert_eq!(vp.x.min, 0.0);
        assert_eq!(vp.x.max, 200.0);
        assert_eq!(vp.y.min, -5.0);
        assert_eq!(vp.y.max, 5.0);

        // Push a single point — x_range would be zero-span without the min-span fix.
        let _ = source.push_y(2.0);
        let vp = plot
            .refresh_viewport(0.0, 0.0)
            .expect("single point viewport");
        // x_range must be valid (positive span).
        assert!(
            vp.x.is_valid(),
            "x range must have positive span: {:?}",
            vp.x
        );
        // y should still come from initial_viewport (no current viewport y yet).
        assert_eq!(vp.y.min, -5.0);
        assert_eq!(vp.y.max, 5.0);
    }

    #[test]
    fn follow_last_n_single_point_x_range_has_min_span() {
        let iv = Viewport::new(Range::new(0.0, 100.0), Range::new(-1.0, 1.0));
        let mut source = Series::line("sensor");
        let mut plot = Plot::builder()
            .view(View::FollowLastN { points: 50 })
            .initial_viewport(iv)
            .build();
        plot.add_series(&source);
        let _ = source.push_y(0.5);

        let vp = plot.refresh_viewport(0.0, 0.0).expect("viewport");
        // min_x_span for points=50 is 49.0, so span must be >= 49.0.
        assert!(
            vp.x.span() >= 49.0,
            "expected x span >= 49 but got {:?}",
            vp.x
        );
    }

    #[test]
    fn follow_last_nxy_uses_initial_viewport_before_data() {
        let iv = Viewport::new(Range::new(0.0, 200.0), Range::new(-3.0, 3.0));
        let mut source = Series::line("sensor");
        let mut plot = Plot::builder()
            .view(View::FollowLastNXY { points: 200 })
            .initial_viewport(iv)
            .build();
        plot.add_series(&source);

        // No data yet.
        let vp = plot.refresh_viewport(0.0, 0.0).expect("initial viewport");
        assert_eq!(vp.x.min, 0.0);
        assert_eq!(vp.x.max, 200.0);
        assert_eq!(vp.y.min, -3.0);
        assert_eq!(vp.y.max, 3.0);

        // Single point — x_range must still be valid.
        let _ = source.push_y(1.0);
        let vp = plot
            .refresh_viewport(0.0, 0.0)
            .expect("single point viewport");
        assert!(
            vp.x.is_valid(),
            "x range must have positive span: {:?}",
            vp.x
        );
    }

    #[test]
    fn series_mut_can_remove_series() {
        let mut first = Series::line("first");
        let mut second = Series::line("second");
        let _ = first.push_y(1.0);
        let _ = second.push_y(9.0);

        let mut plot = Plot::new();
        plot.add_series(&first);
        plot.add_series(&second);

        let removed = plot.series_mut().remove(1);
        assert_eq!(removed.name(), "second");
        assert_eq!(plot.series().len(), 1);
        assert_eq!(plot.series()[0].name(), "first");
    }
}
