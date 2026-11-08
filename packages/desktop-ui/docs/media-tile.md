# MediaTile

A controlled square thumbnail. Supply `source`, `label`, `primary` and `enabled`;
listen with `@primary` and `@remove`. Actions emit intent without changing the
owner's model. Use bounded thumbnails and virtualize collections over20 items.

Hover reveals actions. Keyboard focus reveals and latches them so Tab can reach
the nested buttons without removing the focused subtree; activating the tile
again hides that latch. Leaving/pausing the screen resets interaction state.
Disabled tiles cannot emit mutations. No image cache, timer or listener is added.
