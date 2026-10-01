# Android screens prototype (throwaway)

A throwaway UI prototype for [Android screens](https://github.com/csnook/hab-bot/issues/30). It is kept as a primary source and is not meant to be merged or built on.

- `index.html` is batch 2, the latest: three navigation variants (A, bottom navigation with five views, was chosen), the bottom sheet, the full-screen editor, the notification shade and full-screen alarm, first start with permissions, settings, phones without Google, precise location and the place editor.
- `v1.html` is batch 1, before first start and settings.

Open either file in a browser. They load Preact and htm from cdn.jsdelivr.net and use sample data, with the time fixed at Wednesday 30 September 2026, 17:40. Nothing is saved. Decisions made after the last batch (5 seconds of Undo after a swipe, "This device" instead of "This phone", continuing past refused permissions with a banner) are in the ticket's resolution comment, not in the prototype.
