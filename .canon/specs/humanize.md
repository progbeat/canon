# Humanize

## Duration

`humanize::compact_duration(std::time::Duration) -> String` floors its input to whole seconds.

It renders at most the two highest adjacent units according to this table:

| Duration | Format |
| --- | --- |
| less than one minute | `Ss` |
| one minute to less than one hour | `Mm SSs` |
| one hour to less than one day | `Hh [Mm]` |
| at least one day | `Dd [Hh]` |

`D`, `H`, `M`, and `S` are unpadded decimal values, while `SS` is exactly two digits.
A bracketed zero-valued unit is omitted.
Displayed units are separated by one space, use the suffixes `d`, `h`, `m`, and `s`, and values below the displayed units are discarded without rounding.

```text
1m    -> 1m 00s
1h    -> 1h
1d 1s -> 1d
```
