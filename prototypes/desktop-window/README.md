# Desktop window prototype (throwaway)

A throwaway UI prototype for [Desktop window](https://github.com/csnook/hab-bot/issues/26) and [Desktop dialogs and settings](https://github.com/csnook/hab-bot/issues/29). It is kept on this branch as a primary source and is not meant to be merged or built on.

- `index.html` is version 5, the latest. It has five views (Inbox, Calendar, Agenda, Board, History), the reminder editor in three variants (A was chosen), and Settings with You, Priorities, Sources, This computer, Account, Groups, Server, Calendars and About. It also has sharing, invitations, the first-start flow, snooze all, notices, and the Linux alarm window.
- `v4.html` adds settings, sharing, invitations and first start to version 3.
- `v3.html` adds the History view and the three editor variants to version 2.
- `v2.html` is one window with Inbox, Calendar, Agenda and Board views.
- `v1.html` is the first comparison of three layouts.

Open any file in a browser. They load Preact and htm from cdn.jsdelivr.net and use sample data, with the time fixed at Wednesday 30 September 2026, 17:40. Nothing is saved.

The decisions they settled are in the two tickets' resolution comments.
