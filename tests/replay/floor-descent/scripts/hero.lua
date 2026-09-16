-- Descending a floor, carrying what the adventurer is.
--
-- The point of the fixture is that the swap happens *between* ticks and is
-- reproduced by a replay: the run continues rather than restarting, so the
-- tick counter and the RNG streams keep going, and only what is handed through
-- `carry` survives the boundary.

function on_ready(self)
  local carried = scene.carry()
  -- First floor gets an empty carry; the second gets what the first sent.
  self.hp = carried.hp or 20
  self.depth = carried.depth or 1
  self.floor_name = self:parent():name()
  -- Drawn after the carry is read, so a stream that restarted on the swap
  -- would show up as the same number twice.
  self.roll = rng.range("loot", 1, 1000)
end

function on_tick(self)
  -- Something that visibly does not survive: a variable the next floor's
  -- `on_ready` never sets. A swap that kept the old tree's script state would
  -- carry this across and the probe would catch it.
  self.ticks_here = (self.ticks_here or 0) + 1

  if tick.count() == 5 and self.depth == 1 then
    scene.request_load("floor2", { hp = self.hp - 3, depth = 2 })
  end
end
