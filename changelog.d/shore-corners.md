<!-- markdownlint-disable-file MD041 -->
- **A strip of shore at a tile corner stays reachable.** Where a lake's edge crossed near a tile corner, the tile was water but
  for a strip about a metre wide, and no path node layout fit it: the strip was cut off, and Check paths showed a blocked route
  into it in about 1 in 20 new 16 by 16 Valleys with a lake. Such a tile, and a tile its layout left an island, now takes a
  one-node layout whose crossings (the points 1 to 9 m along an edge the game tries) all land on the strip; the game puts all
  of a one-node tile in one region, so routes reach the strip. Over 64 new 16 by 16 valleys: 0 blocked routes and 0 closed
  passages (were 3 and 41); other samples' routes are unchanged. `LS_NO_LONE` turns it off.
