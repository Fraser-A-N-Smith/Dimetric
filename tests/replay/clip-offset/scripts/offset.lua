-- A clip that does not begin at the start of its strip.
--
-- The simulation keeps two numbers and they are not the same number: the
-- position within the clip, which `anim.frame` answers, and the frame of the
-- whole strip, which lands on the node's `frame` property for the renderer to
-- read. Nothing mapped one to the other, so every clip drew the first frames
-- of its sheet whatever was playing.
--
-- Both are recorded here so a replay pins both.
function on_tick(self)
  local fx = scene.find("/Stage/Fx")
  if tick.count() == 2 then anim.play(fx, "tail") end
  self.clip = fx:get("animation")
  self.within = anim.frame(fx)
  self.sheet = fx:get("frame")
end
