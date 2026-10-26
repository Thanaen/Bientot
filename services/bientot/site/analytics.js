// Audience and loading speed, measured by PostHog EU through e.bientot.dev
// (docs/privacy.md, "The website"). No cookie and no local storage: cookieless_mode counts a visit
// with a daily hash computed by PostHog, which keeps no IP address. Nothing else is recorded.
window.posthog?.init("phc_omakaFDpSAzS5xcHqCbt8fhatcbUF2xgQhz6Tixi872Q", {
  api_host: "https://e.bientot.dev",
  ui_host: "https://eu.posthog.com",
  defaults: "2026-05-30",
  cookieless_mode: "always",
  person_profiles: "identified_only",
  autocapture: false,
  capture_pageview: true,
  capture_pageleave: false,
  capture_dead_clicks: false,
  capture_heatmaps: false,
  capture_exceptions: false,
  capture_performance: { web_vitals: true, network_timing: false },
  disable_session_recording: true,
  disable_surveys: true,
  advanced_disable_flags: true,
});
