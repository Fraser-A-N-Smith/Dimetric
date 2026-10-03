-- Writing a property the node never authored.
--
-- `limits` on a camera and `cone_angle` on a light both have **no default**,
-- so the parser leaves them out of a node that did not write one and there is
-- no authored value for the write to be checked against. Every write below
-- used to land in state as whatever the scripting boundary made of it — a list
-- of integers where a rect belonged, a plain string where an angle did — which
-- drew correctly, hashed, and then failed to come back out of a save.
--
-- So these ticks are the state that was wrong. The hashes beside them are the
-- schema's types, not Lua's shapes.

local view
local lamp

function on_ready(self)
  view = scene.find("/Stage/View")
  lamp = scene.find("/Stage/Lamp")
end

function on_tick(self)
  local t = tick.count() % 4

  -- All three spellings of a rect, in turn, onto a property with no default.
  if t == 0 then
    view:set("limits", { 0, 0, 24, 3 })
  elseif t == 1 then
    view:set("limits", { pos = vec2(-8, -8), size = vec2(48, 24) })
  elseif t == 2 then
    view:set("limits", "[1, 2, 96, 48]")
  else
    -- Read back and written again: a rect reaches a script as a table of
    -- `pos` and `size`, so this is the same type spelled the way it arrived.
    view:set("limits", view:get("limits"))
  end

  -- An angle has no written form but text, and no default here either.
  lamp:set("cone_angle", tostring(20 + t) .. ".5")

  -- Into script state, where the probes can read them and the hash covers
  -- them whether or not a probe does.
  self.limits = view:get("limits").size:x()
  self.cone = lamp:get("cone_angle")
end
