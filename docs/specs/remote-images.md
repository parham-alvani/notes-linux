# Daftar: pictures from the web, only when asked

Shared by Daftar for Linux (Rust, `notes-linux`), macOS (Swift, `notes-mac`) and Android (Kotlin, `notes-droid`). A note can show a picture from another site, `![chart](https://example.com/chart.png)` or an HTML `<img src="https://…">`. Showing it asks that site for it: the site learns that the note was opened, from which address, and when, as a tracking pixel in an email does. Someone who keeps a private vault may not want a note to do that by being read. This setting keeps every picture from the web out of the page until the person asks for it, one at a time.

Pictures from the vault (`![[a.png]]`, `![](uploads/a.png)`) and `data:` pictures are never affected: they ask no one.

## 1. The setting

```toml
[appearance]
remote_images = true
```

`appearance.remote_images` is `true` (the default, and what every app did before: a picture from the web is shown) or `false` (it is not asked for until clicked). Anything else is `true`. It is a line in each app's config template, after `reading_line`:

```toml
# Pictures from the web in a note: true shows them; false asks no site
# for one until you click it, so opening a note tells no one.
remote_images = true
```

A change to the setting applies to the next note shown; the note on screen is drawn again where the app redraws on a settings change.

## 2. The reading page

The page is shared (`web/src`), so the rule is written once:

- The host sends `remoteImages: false` with `daftar.show(note)` when the setting is `false`; absent or `true` changes nothing.
- With it `false`, every `<img>` whose `src` starts with `http://` or `https://` (any case) in the rendered note, in an embedded note, in a hover preview and in a base view's card covers is replaced, before the HTML enters the page, by a placeholder: a `<button class="remote-image" data-src="…" data-alt="…">` that shows a picture icon, the site's host name and the picture's alt text. The `<img>` is never in the document, so nothing is requested.
- A click, Enter or Space on a placeholder puts the `<img>` back, with its `src`, `alt`, `width` and `height`, for that view only: the next time the note is shown it is a placeholder again.
- Links are not affected: `[page](https://…)` stays a link. A picture inside a link (`[![badge](https://…)](https://…)`) becomes a placeholder inside the link; the click on the placeholder loads the picture and does not follow the link.
- The function that does it is `blockRemoteImages(html)` in `render.js`, pure, so it is tested in Node.

## 3. Tests every app must have

The page's tests (`web/test/render.test.mjs`) cover the rewriting:

| Input | `remoteImages` | Result |
| --- | --- | --- |
| `![a](https://x.org/a.png)` | false | a `remote-image` button with `data-src="https://x.org/a.png"`, host `x.org`, no `<img` |
| `![a](https://x.org/a.png)` | true | an `<img>` with that `src` |
| `![[a.png]]` (vault) | false | an `<img>` with a `vault:` `src` |
| `![a](data:image/png;base64,AAAA)` | false | an `<img>` |
| `<img src="HTTP://X.ORG/a.png" width="20">` | false | a placeholder that keeps `width` |
| `[link](https://x.org)` | false | an `<a>`, unchanged |

Each host has a test that the setting is read (`true` when absent or unknown, `false` when written `false`) and that it is sent to the page as `remoteImages`.
