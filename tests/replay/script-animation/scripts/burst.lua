-- Animation playback driven from a script, hashed like any other state.
--
-- Both halves of what used to be broken are here. `anim.play` on an
-- `AnimatedSprite2D` used to last until the next Advance phase, because that
-- phase reads the node's `animation` property and switched back to it. And a
-- finished non-looping clip had no way back to its first frame, so a pooled
-- effect sprite showed its last frame for the life of the node.

local fx

function on_ready(self)
  fx = scene.find("/Stage/Fx")
end

function on_tick(self)
  -- Switched once, from a script, and it has to stay switched.
  if tick.count() == 2 then anim.play(fx, "burst") end

  -- Replayed every time it finishes, which is what a pooled effect does.
  if anim.finished(fx) then
    anim.restart(fx)
    self.replays = (self.replays or 0) + 1
  end

  self.clip = fx:get("animation")
  self.frame = anim.frame(fx)
end
