# Daftar: the holidays of a place, and the day picker

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). Someone who lives in Barcelona plans around Spain's public holidays, Catalonia's and the city's own, and around the feasts that are not days off but fill the streets: the Three Kings' parade, Sant Jordi, Sant Joan's eve, Pride. The apps know these days, show them where a day is shown, and open a note in the vault that says what each one is.

The rules are in the apps, not in the vault: a holiday is the same for everyone in a place, so it is code, with tests, and not a note each person has to keep. What the vault adds is a note per event, found by the event's title, for what the day means to the person.

## 1. The setting

```toml
[calendar]
place = "barcelona"
```

`calendar.place` is `spain`, `catalonia` or `barcelona` (also `es`, `es-ct`, `es-ct-barcelona`), case-insensitive after trimming. Each place keeps the days of the ones before it: Barcelona keeps Catalonia's and Spain's. Empty, or anything else, is none, and nothing is shown. It is a line in each app's config template, under `[calendar]`.

## 2. The events

An event has a title in English (also its note's title), the name the place gives it, the place, whether it is a public holiday, and a rule for its day. The rules are of four kinds:

- the same month and day every year;
- a number of days from Easter Sunday, by the anonymous Gregorian algorithm (Meeus/Jones/Butcher);
- the first to fourth weekday of a month;
- days written for each year (`YYYY-MM-DD`), for an event the place dates anew each year; a year with none written has no event.

The table, in the order a day lists them (H is a holiday):

| Title | Local name | Place | H | Rule |
| --- | --- | --- | --- | --- |
| New Year's Day | Año Nuevo | Spain | H | 1 January |
| Three Kings' Parade | Cabalgata de Reyes | Spain |  | 5 January |
| Three Kings' Day | Día de Reyes | Spain | H | 6 January |
| Santa Eulàlia | Festes de Santa Eulàlia | Barcelona |  | 12 February |
| Dijous Gras | Dijous Gras | Catalonia |  | Easter − 52 |
| Father's Day | Día del Padre | Spain |  | 19 March |
| Good Friday | Viernes Santo | Spain | H | Easter − 2 |
| Easter Monday | Dilluns de Pasqua | Catalonia | H | Easter + 1 |
| Sant Jordi | Diada de Sant Jordi | Catalonia |  | 23 April |
| Labour Day | Día del Trabajador | Spain | H | 1 May |
| Mother's Day | Día de la Madre | Spain |  | first Sunday of May |
| Whit Monday | Dilluns de Pasqua Granada | Barcelona | H | Easter + 50 |
| Sant Joan's Eve | Revetlla de Sant Joan | Catalonia |  | 23 June |
| Sant Joan | Sant Joan | Catalonia | H | 24 June |
| Pride Barcelona | Pride Barcelona | Barcelona |  | 2026-07-18 |
| Assumption Day | Asunción de la Virgen | Spain | H | 15 August |
| La Diada | Diada Nacional de Catalunya | Catalonia | H | 11 September |
| La Mercè | Festes de la Mercè | Barcelona | H | 24 September |
| Spain's National Day | Fiesta Nacional de España | Spain | H | 12 October |
| La Castanyada | La Castanyada | Catalonia |  | 31 October |
| All Saints' Day | Todos los Santos | Spain | H | 1 November |
| Constitution Day | Día de la Constitución | Spain | H | 6 December |
| Immaculate Conception | Inmaculada Concepción | Spain | H | 8 December |
| Christmas Eve | Nochebuena | Spain |  | 24 December |
| Christmas Day | Navidad | Spain | H | 25 December |
| Sant Esteve | Sant Esteve | Catalonia | H | 26 December |
| New Year's Eve | Nochevieja | Spain |  | 31 December |

Catalonia does not move a holiday that falls on a Sunday (in 2027, 15 August and 26 December), so neither do the apps. Barcelona's two local holidays have been Whit Monday and La Mercè every year the city has published; if the city or the Generalitat sets another day, the table changes in all three apps. Pride's day is the Saturday of its demonstration, written each year once it is announced.

Between two days, the events are listed by day, and on one day in the table's order.

## 3. The note

An event's note is the vault's note whose name is the event's title (`Three Kings' Day.md`), found as a `[[Three Kings' Day]]` link from the vault's top would find it. An event with no note is shown without a link. The apps never make the note.

## 4. Where it is shown

- **Today:** a chip for each event of the day (red for a holiday), and a card of the events of the next 45 days: how far each is, its title and local name, and its date; a click opens its note.
- **Home:** the date, followed by the titles of the day's events.
- **The day picker** (section 5): each day with an event is marked.
- **`daftar-cli events`** (Linux): `--from`, `--days` (30 at first) or `--year`, and `--place` over the setting; a line each, `day`, `holiday` or `event`, title, local name and the note's path, by tabs.

## 5. The day picker

Where an app picks a day (a task's dates, a date property), it shows a month of its own instead of the platform's calendar, which knows one calendar and no holidays:

- With `appearance.persian_calendar` off, the Gregorian month, its weeks from Monday, as in Spain, Saturday and Sunday muted.
- With it on, the Jalali month (`daftar_core::jalali`, never the platform's Persian calendar), read from the right, its weeks from Saturday, Friday muted, the title (`آبان ۱۴۰۵`) and each day in Persian digits with its Gregorian day small under it.
- Today is outlined, the picked day filled with the accent, a holiday in red and a day with an event dotted. A day's tooltip says its full date, the Jalali one too in the Jalali month, and its events, a holiday marked.
- Arrows go a month back and on, and Today goes to today's month. It opens on the month of the day in the field, else today's.
- A day picked is written as its Gregorian `YYYY-MM-DD`, in either calendar.

## 6. Tests every app must have

Easter: 2000-04-23, 2024-03-31, 2025-04-20, 2026-04-05, 2027-03-28, 2038-04-25, 2285-03-22.

The place: `barcelona`, ` Catalonia `, `es` are read; `` and `madrid` are none.

| Place, days | Events |
| --- | --- |
| Barcelona, the holidays of 2027 | 01-01, 01-06, 03-26, 03-29, 05-01, 05-17, 06-24, 08-15, 09-11, 09-24, 10-12, 11-01, 12-06, 12-08, 12-25, 12-26 |
| Spain, 2027-03-26 to 03-29 | Good Friday |
| Catalonia, 2027-03-26 to 03-29 | Good Friday, Easter Monday |
| Catalonia, 2027-05-17 | none; Barcelona: Whit Monday |
| Catalonia, February 2027 | Dijous Gras on 02-04 |
| Spain, May 2026; May 2027 | Labour Day 05-01 and Mother's Day 05-03; Labour Day 05-01 and Mother's Day 05-02 |
| Barcelona, 2026-12-24 to 2027-01-06 | Christmas Eve, Christmas Day, Sant Esteve, New Year's Eve, New Year's Day, Three Kings' Parade, Three Kings' Day |
| Barcelona, 2026-07-18; June and July 2027 | Pride Barcelona; no Pride |
| Catalonia, 2026-07-18 | none |

Every title is different from every other.

The month (Linux's `month_grid.rs`, ported with it):

- The Gregorian month of 2026-11-03 is `November 2026`, six days before its 1st (a Sunday), 30 days, 42 cells, weeks from `Mo`.
- The Jalali month of 2026-11-03 is 1405-08, `آبان ۱۴۰۵`, six days before its 1st (2026-10-23, a Friday), 30 days, 2026-11-03 numbered `۱۲`, weeks from `ش`.
- Esfand 1403 has 30 days and is followed by Farvardin 1404; Esfand 1404 has 29. December 2026 is before January 2027; February 2028 has 29 days.
- A month off the Jalali calendar (4000) has no days.
