//! The days a place keeps: its public holidays and the feasts and festivals
//! around them, for Spain, Catalonia and Barcelona
//! (`docs/specs/calendar-events.md`). The rules are written here, not in the
//! vault; each event names a note, by its title, that says what the day is.

use chrono::{Datelike, Weekday};

use crate::day::Day;

/// Where the person lives. Each place keeps the days of the ones before it:
/// Barcelona keeps Catalonia's, and Catalonia keeps Spain's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Place {
    Spain,
    Catalonia,
    Barcelona,
}

impl Place {
    /// `calendar.place` in the config file; empty or anything else is none.
    pub fn parse(text: &str) -> Option<Place> {
        match crate::text::trim(text).to_lowercase().as_str() {
            "spain" | "es" => Some(Spain),
            "catalonia" | "es-ct" => Some(Catalonia),
            "barcelona" | "es-ct-barcelona" => Some(Barcelona),
            _ => None,
        }
    }
}

/// When an event falls in a year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// The same month and day every year.
    Fixed(u32, u32),
    /// Days from Easter Sunday: Good Friday is -2.
    Easter(i64),
    /// The first (1) to fourth weekday of a month.
    Nth(u32, Weekday, u32),
    /// Days set each year, `YYYY-MM-DD`: a year with none has no event.
    Dates(&'static [&'static str]),
}

/// A day of the calendar.
#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    /// In English, and the title of its note.
    pub title: &'static str,
    /// As the place calls it.
    pub local: &'static str,
    pub place: Place,
    /// A public holiday: offices, schools and most shops close.
    pub holiday: bool,
    pub when: When,
}

const fn event(title: &'static str, local: &'static str, place: Place, holiday: bool, when: When) -> Event {
    Event { title, local, place, holiday, when }
}

use Place::{Barcelona, Catalonia, Spain};
use When::{Dates, Easter, Fixed, Nth};

/// Every event, in the order a day lists them. Catalonia keeps every one of
/// Spain's holidays and adds four; Barcelona adds two local ones. A holiday
/// on a Sunday is not moved in Catalonia.
pub const EVENTS: &[Event] = &[
    event("New Year's Day", "Año Nuevo", Spain, true, Fixed(1, 1)),
    event("Three Kings' Parade", "Cabalgata de Reyes", Spain, false, Fixed(1, 5)),
    event("Three Kings' Day", "Día de Reyes", Spain, true, Fixed(1, 6)),
    event("Santa Eulàlia", "Festes de Santa Eulàlia", Barcelona, false, Fixed(2, 12)),
    event("Dijous Gras", "Dijous Gras", Catalonia, false, Easter(-52)),
    event("Father's Day", "Día del Padre", Spain, false, Fixed(3, 19)),
    event("Good Friday", "Viernes Santo", Spain, true, Easter(-2)),
    event("Easter Monday", "Dilluns de Pasqua", Catalonia, true, Easter(1)),
    event("Sant Jordi", "Diada de Sant Jordi", Catalonia, false, Fixed(4, 23)),
    event("Labour Day", "Día del Trabajador", Spain, true, Fixed(5, 1)),
    event("Mother's Day", "Día de la Madre", Spain, false, Nth(5, Weekday::Sun, 1)),
    event("Whit Monday", "Dilluns de Pasqua Granada", Barcelona, true, Easter(50)),
    event("Sant Joan's Eve", "Revetlla de Sant Joan", Catalonia, false, Fixed(6, 23)),
    event("Sant Joan", "Sant Joan", Catalonia, true, Fixed(6, 24)),
    event("Pride Barcelona", "Pride Barcelona", Barcelona, false, Dates(&["2026-07-18"])),
    event("Assumption Day", "Asunción de la Virgen", Spain, true, Fixed(8, 15)),
    event("La Diada", "Diada Nacional de Catalunya", Catalonia, true, Fixed(9, 11)),
    event("La Mercè", "Festes de la Mercè", Barcelona, true, Fixed(9, 24)),
    event("Spain's National Day", "Fiesta Nacional de España", Spain, true, Fixed(10, 12)),
    event("La Castanyada", "La Castanyada", Catalonia, false, Fixed(10, 31)),
    event("All Saints' Day", "Todos los Santos", Spain, true, Fixed(11, 1)),
    event("Constitution Day", "Día de la Constitución", Spain, true, Fixed(12, 6)),
    event("Immaculate Conception", "Inmaculada Concepción", Spain, true, Fixed(12, 8)),
    event("Christmas Eve", "Nochebuena", Spain, false, Fixed(12, 24)),
    event("Christmas Day", "Navidad", Spain, true, Fixed(12, 25)),
    event("Sant Esteve", "Sant Esteve", Catalonia, true, Fixed(12, 26)),
    event("New Year's Eve", "Nochevieja", Spain, false, Fixed(12, 31)),
];

/// An event on its day.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Happening {
    pub day: Day,
    pub event: &'static Event,
}

pub struct CalendarEvents;

impl CalendarEvents {
    /// Easter Sunday of [year] in the Gregorian calendar (the anonymous
    /// Gregorian algorithm, Meeus/Jones/Butcher).
    pub fn easter(year: i32) -> Day {
        let a = year % 19;
        let b = year / 100;
        let c = year % 100;
        let d = b / 4;
        let e = b % 4;
        let f = (b + 8) / 25;
        let g = (b - f + 1) / 3;
        let h = (19 * a + b - d - g + 15) % 30;
        let i = c / 4;
        let k = c % 4;
        let l = (32 + 2 * e + 2 * i - h - k) % 7;
        let m = (a + 11 * h + 22 * l) / 451;
        let month = (h + l - 7 * m + 114) / 31;
        let day = (h + l - 7 * m + 114) % 31 + 1;
        Day::new(year, month as u32, day as u32).expect("Easter is a day")
    }

    /// The day [event] falls on in [year], if it does that year.
    pub fn day(event: &Event, year: i32) -> Option<Day> {
        match event.when {
            Fixed(month, day) => Day::new(year, month, day),
            Easter(offset) => Some(CalendarEvents::easter(year).adding_days(offset)),
            Nth(month, weekday, nth) => {
                let first = Day::new(year, month, 1)?;
                let ahead = (7 + weekday.num_days_from_monday() - first.date().weekday().num_days_from_monday()) % 7;
                let day = first.adding_days(i64::from(ahead + 7 * (nth - 1)));
                (day.month() == month).then_some(day)
            }
            Dates(dates) => dates.iter().filter_map(|d| Day::parse(d)).find(|d| d.year() == year),
        }
    }

    /// The events [place] keeps from [from] to [to], both kept, by day and
    /// then in the table's order.
    pub fn between(place: Place, from: Day, to: Day) -> Vec<Happening> {
        let mut found = Vec::new();
        for year in from.year()..=to.year() {
            for event in EVENTS.iter().filter(|e| e.place <= place) {
                if let Some(day) = CalendarEvents::day(event, year)
                    && from <= day
                    && day <= to
                {
                    found.push(Happening { day, event });
                }
            }
        }
        found.sort_by_key(|h| (h.day, EVENTS.iter().position(|e| std::ptr::eq(e, h.event))));
        found
    }

    /// The events of one day.
    pub fn on(place: Place, day: Day) -> Vec<&'static Event> {
        CalendarEvents::between(place, day, day).into_iter().map(|h| h.event).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        Day::parse(text).unwrap()
    }

    fn titles(place: Place, from: &str, to: &str) -> Vec<String> {
        CalendarEvents::between(place, day(from), day(to)).iter().map(|h| format!("{} {}", h.day, h.event.title)).collect()
    }

    #[test]
    fn easter_falls_where_the_church_puts_it() {
        for (year, easter) in [
            (2000, "2000-04-23"),
            (2024, "2024-03-31"),
            (2025, "2025-04-20"),
            (2026, "2026-04-05"),
            (2027, "2027-03-28"),
            (2038, "2038-04-25"),
            (2285, "2285-03-22"),
        ] {
            assert_eq!(CalendarEvents::easter(year), day(easter), "{year}");
        }
    }

    #[test]
    fn a_place_is_read_from_the_config() {
        assert_eq!(Place::parse("barcelona"), Some(Barcelona));
        assert_eq!(Place::parse(" Catalonia "), Some(Catalonia));
        assert_eq!(Place::parse("es"), Some(Spain));
        assert_eq!(Place::parse(""), None);
        assert_eq!(Place::parse("madrid"), None);
    }

    #[test]
    fn barcelona_2027_has_the_official_holidays() {
        let holidays: Vec<String> = CalendarEvents::between(Barcelona, day("2027-01-01"), day("2027-12-31"))
            .iter()
            .filter(|h| h.event.holiday)
            .map(|h| h.day.to_string())
            .collect();
        assert_eq!(
            holidays,
            [
                "2027-01-01",
                "2027-01-06",
                "2027-03-26",
                "2027-03-29",
                "2027-05-01",
                "2027-05-17",
                "2027-06-24",
                "2027-08-15",
                "2027-09-11",
                "2027-09-24",
                "2027-10-12",
                "2027-11-01",
                "2027-12-06",
                "2027-12-08",
                "2027-12-25",
                "2027-12-26",
            ]
        );
    }

    #[test]
    fn a_place_keeps_the_days_of_the_places_around_it() {
        assert_eq!(titles(Spain, "2027-03-26", "2027-03-29"), ["2027-03-26 Good Friday"]);
        assert_eq!(titles(Catalonia, "2027-03-26", "2027-03-29"), ["2027-03-26 Good Friday", "2027-03-29 Easter Monday"]);
        assert!(titles(Catalonia, "2027-05-17", "2027-05-17").is_empty());
        assert_eq!(titles(Barcelona, "2027-05-17", "2027-05-17"), ["2027-05-17 Whit Monday"]);
    }

    #[test]
    fn days_from_easter_and_weekdays_move_with_the_year() {
        assert_eq!(titles(Catalonia, "2027-02-01", "2027-02-28"), ["2027-02-04 Dijous Gras"]);
        assert_eq!(titles(Spain, "2026-05-01", "2026-05-31"), ["2026-05-01 Labour Day", "2026-05-03 Mother's Day"]);
        assert_eq!(titles(Spain, "2027-05-01", "2027-05-31"), ["2027-05-01 Labour Day", "2027-05-02 Mother's Day"]);
    }

    #[test]
    fn a_span_runs_over_the_new_year_in_order() {
        assert_eq!(
            titles(Barcelona, "2026-12-24", "2027-01-06"),
            [
                "2026-12-24 Christmas Eve",
                "2026-12-25 Christmas Day",
                "2026-12-26 Sant Esteve",
                "2026-12-31 New Year's Eve",
                "2027-01-01 New Year's Day",
                "2027-01-05 Three Kings' Parade",
                "2027-01-06 Three Kings' Day",
            ]
        );
    }

    #[test]
    fn a_dated_event_is_only_in_its_years() {
        assert_eq!(titles(Barcelona, "2026-07-18", "2026-07-18"), ["2026-07-18 Pride Barcelona"]);
        assert!(titles(Barcelona, "2027-06-01", "2027-07-31").iter().all(|t| !t.contains("Pride")));
        assert!(titles(Catalonia, "2026-07-18", "2026-07-18").is_empty());
    }

    #[test]
    fn one_day_lists_its_events() {
        let on = CalendarEvents::on(Barcelona, day("2027-06-24"));
        assert_eq!(on.iter().map(|e| (e.title, e.holiday)).collect::<Vec<_>>(), [("Sant Joan", true)]);
        assert!(CalendarEvents::on(Barcelona, day("2027-06-25")).is_empty());
    }

    #[test]
    fn titles_are_unique() {
        let mut titles: Vec<&str> = EVENTS.iter().map(|e| e.title).collect();
        titles.sort_unstable();
        titles.dedup();
        assert_eq!(titles.len(), EVENTS.len());
    }
}
