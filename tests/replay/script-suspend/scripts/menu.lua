-- Asking to be suspended, in a run that is being replayed.
--
-- A replay must ignore `app.suspend()` and `app.resume()` and answer
-- `app.suspended()` false, for the same reason it ignores `app.quit()`: a
-- recorded run that depended on a file beside it would reproduce only on the
-- machine that made one, and a run that was suspended has to hash identically
-- to one whose window was closed.
--
-- So every call below is made every tick, and the hashes beside this file are
-- the hashes of a run that asked for none of it.

function on_tick(self)
  local mark = scene.find("/Stage/Mark")

  -- Work, so there is a run to tell apart from a stalled one.
  self.total = (self.total or 0) + rng.range("stride", 1, 6)
  mark.pos = mark.pos + vec2(1, 0)

  -- The answer a replay has to give, recorded in state so a probe can read it
  -- and the hash covers it whether a probe does or not.
  self.waiting = app.suspended()

  -- And the requests, which go nowhere here. A game would reach these from a
  -- menu; a replay drops them on the floor.
  if tick.count() % 3 == 0 then
    app.discard_suspended()
  end
  if tick.count() == 10 then
    app.suspend()
  end
  if tick.count() == 20 then
    app.resume()
  end
end
