# Validate the daily workflow

Run build and tests sequentially.

```powershell
dotnet restore WorkTime.slnx
dotnet build WorkTime.slnx -c Release --no-restore
dotnet test WorkTime.slnx -c Release --no-build --no-restore --filter 'Category!=Live'
```

Launch WorkTime with `--demo --data-dir <isolated-directory>`. Restart with
the same demo flag and folder: the demo keeps its settings and corrections there,
and seeds its scenario only into a new folder. It refuses a folder that holds
other WorkTime data or another scenario's demo. Add `--english --light` for
English and light-theme validation.

1. In Schedule, enter `7:24` and inspect the target after save and restart.
2. Enter malformed values and verify the draft error and unchanged saved value.
3. Restart with `--demo-problem-periods --data-dir <another-isolated-directory>` to seed suspension and multiple gaps. Open the source warning's history action.
4. Correct one exact period, supply a reason, and inspect the remaining periods.
5. Restart and verify corrected daily and weekly totals and retained reason.
6. Compare morning guidance with comfortable, little and insufficient margin.
7. Pause and finish tracking. Verify that no active-work assurance remains.
8. Use the keyboard and a short window to reach all actions in both languages.

Record actual outcomes in docs/daily-workflow-validation.md. Do not mark a native
or restart scenario passed from source inspection alone.
