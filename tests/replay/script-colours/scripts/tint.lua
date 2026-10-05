-- A colour is simulation state, so it hashes and replays like a position.
--
-- Three of the four types that reach a script as a string are here on
-- purpose: a colour, an enum and an angle. Each is read back and written
-- again, which used to be refused as a type change and is the reason a script
-- could not tint anything at all.

-- Looked up in the hook that needs them rather than cached in a file-scope
-- local, which is what `DIM0508` is about: a local lives in Lua, and a host
-- that restores a run it did not play has not run `on_ready` for these nodes.
function on_tick(self)
  local lamp = scene.find("/Stage/Lamp")
  local panel = scene.find("/Stage/Backdrop")
  local t = tick.count() % 256

  -- Constructed, from bytes. No float goes near this.
  lamp:set("color", color.rgba(t, 255 - t, 128, 255))

  -- Read back and written again, as text, on three different types.
  lamp:set("shape", lamp:get("shape"))
  lamp:set("cone_angle", lamp:get("cone_angle"))
  panel:set("modulate", panel:get("modulate"))

  -- Into state, where the probes can see them and the hash covers them.
  local c = lamp:get("color")
  self.hex = c
  self.r = color.parse(c):r()
  self.g = color.parse(c):g()
  self.b = color.parse(c):b()
  self.shape = lamp:get("shape")
  self.backdrop = panel:get("modulate")
end
