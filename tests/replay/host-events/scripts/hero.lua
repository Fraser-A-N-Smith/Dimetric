-- Telling the host things, and the guarantee that saying them changes nothing.
--
-- The fixture exists to pin that a run which emits achievements hashes
-- identically to one that does not. If emitting consumed a random number or
-- wrote something hashed, a build with Steam disabled would diverge from one
-- with it on -- which is the sound list's argument, word for word.
--
-- The script deliberately emits on most ticks and draws from a seeded stream
-- either side of the emit, so a `event.emit` that touched the RNG would show up
-- here as a divergence rather than three weeks later on somebody's machine.

function on_ready(self)
  self.rolls = 0
  self.rank = 0
  event.emit("presence", { where = "crypt", depth = 1 })
end

function on_tick(self)
  -- A draw before, so the stream position is observable.
  local before = rng.range("loot", 1, 1000)

  if tick.count() % 3 == 0 then
    event.emit("achievement", { id = "tick_" .. tick.count(), rolls = self.rolls })
  end

  -- A rank ladder, which is the shape the game actually has: integers in
  -- script variables, with the host told only when one is crossed.
  self.rolls = self.rolls + 1
  if self.rolls % 5 == 0 then
    self.rank = self.rank + 1
    event.emit("rank", { reached = self.rank })
    -- An ordered payload, because a list that came back reordered would be a
    -- host acting on the wrong thing (I4).
    event.emit("loadout", { slots = { "amber", "jet", "opal" } })
  end

  -- And a draw after. `before` and `after` are both recorded so the fixture's
  -- probes can assert the stream advanced by exactly two per tick.
  local after = rng.range("loot", 1, 1000)
  self.last_before = before
  self.last_after = after
end
