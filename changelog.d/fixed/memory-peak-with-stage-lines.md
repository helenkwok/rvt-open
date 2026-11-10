- **The Memory workflow's peak column is the run's peak again.**
  Printing the per-stage memory resets the kernel's peak at every stage, so
  the time measurement it also reads saw only the last stage's peak, which is
  the memory still held at the end; the column now takes the larger of that
  and the largest stage peak. On the BIM4LCA concrete housing model the
  column read 1,215 and 1,852 MiB for a run whose peak is 3,590 MiB both times.
- **The Memory workflow fails only on a difference nobody meant.** A head
  variant that only changes how the build runs must still write exactly what
  the head build writes; the head build itself may differ from the base on a
  pull request or with `outputs_may_change`, where the difference is reported
  instead of failing every model.
