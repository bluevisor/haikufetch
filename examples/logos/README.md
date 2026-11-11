# Custom logos

A logo is a text file. Put it in `~/.config/haikufetch/logos/` (`~/config/settings/haikufetch/logos/`
on Haiku) and pick it by name, or point `[logo] name` at any path.

`$1` to `$9` switch to the 1st to 9th of `[logo] colors`; text before the first marker takes
color 1. Without `colors` the art is drawn white, and labels set to `key_color = "auto"` take
color 1. A file may also carry its own escape codes instead.

## Example: the Seen Health ginkgo

`seen.txt` here is the three-leaf ginkgo, each leaf in its own brand color. Blocks the leaf only
partly covers use a darker copy of its color (`$4` to `$6`), which softens the stair-step edges:

```sh
mkdir -p ~/.config/haikufetch/logos
cp examples/logos/seen.txt ~/.config/haikufetch/logos/
```

```toml
[logo]
name = "seen"
# top-left, right and bottom leaves, then darker copies for their edges
colors = ["#a7e2c0", "#5dc6b2", "#00a7a2", "#759e86", "#418b7d", "#007571"]

[style]
key_color = "#5dc6b2"                          # the right leaf's teal
```
