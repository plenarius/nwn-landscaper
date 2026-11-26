<!-- markdownlint-disable-file MD041 -->
- **Routes too long for one click are no longer called blocked.** The path check now tries shorter clicks along each route
  that stops short. When they get there, the route goes in the report's new `long_routes` list (with how many clicks) and
  `routes.long` counts it, instead of a `failed_route` warning. NWN's route search gives up when the way is about 20 tiles
  longer than going straight there, so a new 12 by 12 *Descending cave* showed 68 blocked routes though every one is walkable
  in two or three clicks; the game client stops and arrives exactly where the check says. `check` and `export` print the
  count after the routes.
