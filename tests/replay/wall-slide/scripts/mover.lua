-- Pushes into the corner at a constant velocity. The interesting part is what
-- the engine does about it, not what this script does.
function on_ready(self)
  self:set_velocity(vec2(fx.new(400), fx.new(300)))
end
