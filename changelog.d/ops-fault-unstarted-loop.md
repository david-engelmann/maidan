### A fault armed for a loop that never started says so

- **Fixed:** when `MAIDAN_OPS_FAULT` names a loop that has not started by the first watchdog pass (a misspelt name, or a loop this deployment does not run), the operator channel gets one warning naming it. Before, the drill waited for an alert that could never come.
