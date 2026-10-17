use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::Serialize;
use url::Url;

use crate::types::library::LibraryItem;

/// Number of UTC days covered by [`LibraryStats::daily_activity`], today included.
pub const DAILY_ACTIVITY_DAYS: u32 = 30;
/// Maximum number of entries in [`LibraryStats::top`].
pub const TOP_ITEMS_COUNT: usize = 5;

/// Viewing statistics derived from the library items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryStats {
    /// Sum of `overall_time_watched` over the included items, in milliseconds.
    pub total_watched_ms: u64,
    /// Movies with `times_watched > 0` or `flagged_watched > 0`.
    pub movies_watched: u32,
    /// Series with any watch activity.
    pub series_count: u32,
    /// Number of included items per `type`, ordered by `type` (alphabetically).
    pub by_type: Vec<TypeCount>,
    /// Up to [`TOP_ITEMS_COUNT`] items with the most `overall_time_watched`,
    /// descending. Items with no watch time are not listed.
    pub top: Vec<TopItem>,
    /// Exactly [`DAILY_ACTIVITY_DAYS`] entries, oldest first, ending with the
    /// current UTC day. Zero days are included.
    pub daily_activity: Vec<DailyActivity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeCount {
    pub r#type: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TopItem {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub poster: Option<Url>,
    pub overall_time_watched_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    /// UTC date, serialized as `YYYY-MM-DD`.
    pub date: NaiveDate,
    /// Number of included items whose `last_watched` falls on this UTC day.
    pub count: u32,
}

/// Whether the item shows any sign of having been watched.
fn has_watch_activity(item: &LibraryItem) -> bool {
    let state = &item.state;
    state.overall_time_watched > 0
        || state.time_watched > 0
        || state.times_watched > 0
        || state.flagged_watched > 0
        // Episodes or seasons marked as watched only set the bitfield
        // (and `last_watched`), without touching the counters above.
        || state.watched.is_some()
}

/// Items that take part in the statistics.
///
/// Removed or temporary items are ignored unless they have watch activity:
/// Stremio keeps watched items that were removed from the library with
/// `removed = true`, while temporary items created by just browsing carry
/// no watch data at all.
fn is_included(item: &LibraryItem) -> bool {
    (!item.removed && !item.temp) || has_watch_activity(item)
}

/// Computes the viewing statistics for the given library items.
///
/// `now` defines the 30-day window of [`LibraryStats::daily_activity`]
/// (UTC days, `now`'s day included). It is taken as a parameter so the
/// function stays pure and testable.
pub fn library_stats<'a>(
    items: impl Iterator<Item = &'a LibraryItem>,
    now: DateTime<Utc>,
) -> LibraryStats {
    let today = now.date_naive();
    let first_day = today - Duration::days(i64::from(DAILY_ACTIVITY_DAYS) - 1);

    let mut stats = LibraryStats::default();
    let mut by_type: BTreeMap<&str, u32> = BTreeMap::new();
    let mut daily_counts: HashMap<NaiveDate, u32> = HashMap::new();
    let mut top_candidates: Vec<&LibraryItem> = Vec::new();

    for item in items.filter(|item| is_included(item)) {
        let state = &item.state;

        stats.total_watched_ms = stats
            .total_watched_ms
            .saturating_add(state.overall_time_watched);

        *by_type.entry(item.r#type.as_str()).or_default() += 1;

        match item.r#type.as_str() {
            "movie" if state.times_watched > 0 || state.flagged_watched > 0 => {
                stats.movies_watched += 1;
            }
            "series" if has_watch_activity(item) => {
                stats.series_count += 1;
            }
            _ => {}
        }

        if let Some(last_watched) = state.last_watched {
            let day = last_watched.date_naive();
            if day >= first_day && day <= today {
                *daily_counts.entry(day).or_default() += 1;
            }
        }

        if state.overall_time_watched > 0 {
            top_candidates.push(item);
        }
    }

    stats.by_type = by_type
        .into_iter()
        .map(|(r#type, count)| TypeCount {
            r#type: r#type.to_owned(),
            count,
        })
        .collect();

    top_candidates.sort_by(|a, b| {
        b.state
            .overall_time_watched
            .cmp(&a.state.overall_time_watched)
            .then_with(|| a.id.cmp(&b.id))
    });
    stats.top = top_candidates
        .into_iter()
        .take(TOP_ITEMS_COUNT)
        .map(|item| TopItem {
            id: item.id.to_owned(),
            name: item.name.to_owned(),
            r#type: item.r#type.to_owned(),
            poster: item.poster.to_owned(),
            overall_time_watched_ms: item.state.overall_time_watched,
        })
        .collect();

    stats.daily_activity = (0..DAILY_ACTIVITY_DAYS)
        .map(|offset| {
            let date = first_day + Duration::days(i64::from(offset));
            DailyActivity {
                date,
                count: daily_counts.get(&date).copied().unwrap_or(0),
            }
        })
        .collect();

    stats
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::types::library::LibraryItemState;
    use crate::types::resource::{MetaItemBehaviorHints, PosterShape};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap()
    }

    fn item(id: &str, r#type: &str) -> LibraryItem {
        LibraryItem {
            id: id.to_owned(),
            name: format!("Name {id}"),
            r#type: r#type.to_owned(),
            poster: None,
            poster_shape: PosterShape::Poster,
            removed: false,
            temp: false,
            ctime: None,
            mtime: now(),
            state: LibraryItemState::default(),
            behavior_hints: MetaItemBehaviorHints::default(),
        }
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, s).unwrap()
    }

    fn count_on(stats: &LibraryStats, date: &str) -> Option<u32> {
        stats
            .daily_activity
            .iter()
            .find(|day| day.date.to_string() == date)
            .map(|day| day.count)
    }

    #[test]
    fn empty_library_yields_zeroed_stats_with_full_window() {
        let stats = library_stats(std::iter::empty(), now());

        assert_eq!(stats.total_watched_ms, 0);
        assert_eq!(stats.movies_watched, 0);
        assert_eq!(stats.series_count, 0);
        assert!(stats.by_type.is_empty());
        assert!(stats.top.is_empty());
        assert_eq!(stats.daily_activity.len(), DAILY_ACTIVITY_DAYS as usize);
        assert!(stats.daily_activity.iter().all(|day| day.count == 0));
        assert_eq!(
            stats.daily_activity.first().map(|day| day.date.to_string()),
            Some("2026-09-18".to_owned())
        );
        assert_eq!(
            stats.daily_activity.last().map(|day| day.date.to_string()),
            Some("2026-10-17".to_owned())
        );
    }

    #[test]
    fn top_is_five_items_by_overall_time_watched_descending() {
        let times = [
            ("a", 5),
            ("b", 9),
            ("c", 1),
            ("d", 9),
            ("e", 3),
            ("f", 0),
            ("g", 7),
        ];
        let items: Vec<LibraryItem> = times
            .iter()
            .map(|(id, overall)| {
                let mut item = item(id, "movie");
                item.state.overall_time_watched = *overall;
                item
            })
            .collect();

        let stats = library_stats(items.iter(), now());

        let top_ids: Vec<&str> = stats.top.iter().map(|top| top.id.as_str()).collect();
        // Ties are broken by id, zero-time items are not listed.
        assert_eq!(top_ids, vec!["b", "d", "g", "a", "e"]);
        assert_eq!(stats.top[0].overall_time_watched_ms, 9);
        assert_eq!(stats.top[0].name, "Name b");
        assert_eq!(stats.total_watched_ms, 34);
    }

    #[test]
    fn daily_window_includes_first_and_last_day_and_excludes_outside() {
        let mut in_window_start = item("start", "movie");
        in_window_start.state.times_watched = 1;
        in_window_start.state.last_watched = Some(at(2026, 9, 18, 0, 0, 0));

        let mut before_window = item("before", "movie");
        before_window.state.times_watched = 1;
        before_window.state.last_watched = Some(at(2026, 9, 17, 23, 59, 59));

        let mut today_start = item("today_start", "movie");
        today_start.state.times_watched = 1;
        today_start.state.last_watched = Some(at(2026, 10, 17, 0, 0, 0));

        let mut today_now = item("today_now", "movie");
        today_now.state.times_watched = 1;
        today_now.state.last_watched = Some(now());

        let mut future = item("future", "movie");
        future.state.times_watched = 1;
        future.state.last_watched = Some(at(2026, 10, 18, 0, 0, 0));

        let items = [
            in_window_start,
            before_window,
            today_start,
            today_now,
            future,
        ];
        let stats = library_stats(items.iter(), now());

        assert_eq!(stats.daily_activity.len(), DAILY_ACTIVITY_DAYS as usize);
        assert_eq!(count_on(&stats, "2026-09-18"), Some(1));
        assert_eq!(count_on(&stats, "2026-10-17"), Some(2));
        assert_eq!(count_on(&stats, "2026-09-17"), None);
        assert_eq!(count_on(&stats, "2026-10-18"), None);
        assert_eq!(count_on(&stats, "2026-10-01"), Some(0));
        let total: u32 = stats.daily_activity.iter().map(|day| day.count).sum();
        assert_eq!(total, 3);
    }

    #[test]
    fn removed_and_temp_items_are_ignored_unless_they_have_watch_activity() {
        // Watched, then removed from the library: counted.
        let mut removed_watched = item("removed_watched", "movie");
        removed_watched.removed = true;
        removed_watched.state.overall_time_watched = 1000;
        removed_watched.state.last_watched = Some(at(2026, 10, 10, 20, 0, 0));

        // Temporary item created by browsing (no watch data): ignored, even
        // though `last_watched` is set on creation.
        let mut temp_browsed = item("temp_browsed", "movie");
        temp_browsed.removed = true;
        temp_browsed.temp = true;
        temp_browsed.state.last_watched = Some(now());

        // Temporary item that was actually watched: counted.
        let mut temp_watched = item("temp_watched", "movie");
        temp_watched.temp = true;
        temp_watched.state.times_watched = 1;
        temp_watched.state.last_watched = Some(now());

        // Removed series with watch time: counted as a series.
        let mut removed_series = item("removed_series", "series");
        removed_series.removed = true;
        removed_series.state.time_watched = 500;
        removed_series.state.overall_time_watched = 2000;

        // Regular library item without any activity: counted in by_type only.
        let regular = item("regular", "movie");

        let items = [
            removed_watched,
            temp_browsed,
            temp_watched,
            removed_series,
            regular,
        ];
        let stats = library_stats(items.iter(), now());

        assert_eq!(
            stats.by_type,
            vec![
                TypeCount {
                    r#type: "movie".to_owned(),
                    count: 3,
                },
                TypeCount {
                    r#type: "series".to_owned(),
                    count: 1,
                },
            ]
        );
        // Only `temp_watched` has a watch counter; `removed_watched` has
        // watch time but was never completed.
        assert_eq!(stats.movies_watched, 1);
        assert_eq!(stats.series_count, 1);
        assert_eq!(stats.total_watched_ms, 3000);
        let top_ids: Vec<&str> = stats.top.iter().map(|top| top.id.as_str()).collect();
        assert_eq!(top_ids, vec!["removed_series", "removed_watched"]);
        // `temp_browsed` must not show up in the activity.
        assert_eq!(count_on(&stats, "2026-10-17"), Some(1));
        assert_eq!(count_on(&stats, "2026-10-10"), Some(1));
    }

    #[test]
    fn serializes_with_camel_case_keys_for_the_web() {
        let mut watched = item("tt1", "movie");
        watched.state.times_watched = 1;
        watched.state.overall_time_watched = 42;
        watched.state.last_watched = Some(now());
        watched.poster = Some(Url::parse("https://example.com/poster.jpg").unwrap());

        let stats = library_stats(std::iter::once(&watched), now());
        let json = serde_json::to_value(&stats).expect("serializable");

        assert_eq!(json["totalWatchedMs"], 42);
        assert_eq!(json["moviesWatched"], 1);
        assert_eq!(json["seriesCount"], 0);
        assert_eq!(json["byType"][0]["type"], "movie");
        assert_eq!(json["byType"][0]["count"], 1);
        assert_eq!(json["top"][0]["id"], "tt1");
        assert_eq!(json["top"][0]["overallTimeWatchedMs"], 42);
        assert_eq!(json["top"][0]["poster"], "https://example.com/poster.jpg");
        assert_eq!(json["dailyActivity"][29]["date"], "2026-10-17");
        assert_eq!(json["dailyActivity"][29]["count"], 1);
        assert_eq!(json["dailyActivity"].as_array().map(Vec::len), Some(30));
    }
}
