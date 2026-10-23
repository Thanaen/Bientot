# Data model

No persistent schema changes are needed.

- SettingsNumberParser converts H:mm text to the existing TimeSpan values.
- HistoryBreakdownItem retains localized display fields and an optional exact
  correction interval derived from the loaded summary.
- AdjustmentDialogRequest and AdjustTimeDialogContext accept an optional initial
  interval for a create operation. An edit carries its existing adjustment.
- The dialog retains original UTC endpoints and initial displayed text. Unedited
  endpoints preserve the original instant. Edited endpoints follow validation.
- ProjectionPendingValue can carry an estimated qualifier. IndicatorState and
  ForecastAvailability remain the source of classification truth.
- ManualOrigin continues to point to the adjustment and original automatic origin.
