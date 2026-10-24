# Fullscreen when Cubby is wrapped in a native iOS app

Cubby's `/watch` fullscreen toggle puts Cubby's own player container (embed, controls,
Resume screen, end screen) into fullscreen. It never fullscreens the YouTube iframe, and
YouTube's own fullscreen stays off (`fs=0`).

- **True element fullscreen:** in a `WKWebView` wrapper set
  `configuration.preferences.isElementFullscreenEnabled = true` (iPadOS 16.4+).
- **Without it** (or on iPhone Safari) `document.fullscreenEnabled` is false and the toggle
  switches to a CSS mode: the container is `position: fixed`, `100dvw x 100dvh`, with
  safe-area insets respected. In a full-screen native wrapper that is equivalent.

Both modes keep every safeguard: PAUSED hides the embed behind the Resume screen, ENDED
shows only the end screen, an off-list video id ends the grant.
