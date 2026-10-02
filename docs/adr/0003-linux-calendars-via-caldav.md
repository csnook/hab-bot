# On Linux, calendars are read over CalDAV, not from the desktop's calendar store

Although KDE Plasma is the primary desktop, the app reads calendars on Linux with its own Rust CalDAV client, signing in to Google through the system browser, rather than from KDE's Akonadi store or GNOME's Evolution Data Server. Akonadi is only reachable through C++/Qt libraries, which would break the rule of using only Rust and TypeScript, and EDS's D-Bus interface is marked private and is usually absent on Plasma. CalDAV also works unchanged for other calendar servers later. On Android the app still reads the system calendar provider, which needs no Google project.

## Considered Options

- **Akonadi**: the natural choice on Plasma, and it already holds the user's Google account, but it needs a C++ bridge.
- **Evolution Data Server over D-Bus**: reachable from Rust with zbus, but the interface is private and EDS is GNOME's stack.
- **Google Calendar API**: Google-only, so every other provider would need its own client.

## Consequences

- The app needs its own Google Cloud project and OAuth client, with Google's warnings for unverified apps that read calendars. It ships with the owner's client, and a setting lets anyone self-hosting use their own.
- The user signs in to Google separately in the app, even if Plasma is already signed in.
