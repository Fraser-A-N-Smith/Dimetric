-- A floor built from the run's seed, which is what a roguelike does and what
-- authoring rooms as files cannot.
--
-- The fixture exists to pin that the grid a script paints is part of the state
-- hash and comes out identical on every machine: the walk below draws its every
-- decision from a seeded stream, so a platform that disagreed about tiles would
-- diverge here rather than three weeks later in somebody's game.

local WALL, FLOOR, RUBBLE = 1, 2, 3
local W, H = 24, 18

function on_ready(self)
  local layer = scene.find("/World/Floor")

  -- Solid rock, then a room carved out of it.
  tiles.fill(layer, 0, 0, W, H, WALL)
  tiles.fill(layer, 1, 1, W - 2, H - 2, FLOOR)

  -- A drunkard's walk scattering rubble. Every step is an integer draw from a
  -- named stream, so the same seed lays the same rubble down.
  local x, y = W // 2, H // 2
  for _ = 1, 60 do
    local step = rng.range("floor", 0, 3)
    if step == 0 then x = x + 1
    elseif step == 1 then x = x - 1
    elseif step == 2 then y = y + 1
    else y = y - 1 end
    -- Clamped inside the walls rather than wrapped: a walk that left the room
    -- would paint rubble into rock nobody can reach.
    if x < 1 then x = 1 elseif x > W - 2 then x = W - 2 end
    if y < 1 then y = 1 elseif y > H - 2 then y = H - 2 end
    tiles.set(layer, x, y, RUBBLE)
  end
end

function on_tick(self)
  -- Read the grid back on the tick after it was written, and count what the
  -- generator actually produced. A probe can then assert on the numbers, which
  -- is a stronger claim than "the hash matched": it says the tiles are the
  -- tiles, not merely that they are consistently wrong.
  if tick.count() ~= 1 then return end
  local layer = scene.find("/World/Floor")
  local walls, floors, rubble = 0, 0, 0
  for cy = 0, H - 1 do
    for cx = 0, W - 1 do
      local t = tiles.get(layer, cx, cy)
      if t == WALL then walls = walls + 1
      elseif t == FLOOR then floors = floors + 1
      elseif t == RUBBLE then rubble = rubble + 1 end
    end
  end
  self.walls = walls
  self.floors = floors
  self.rubble = rubble

  local b = tiles.bounds(layer)
  self.bounds_w = b.w
  self.bounds_h = b.h
end
