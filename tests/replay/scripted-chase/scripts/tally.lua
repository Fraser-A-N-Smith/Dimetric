-- Counts arrivals, so the replay has a scalar to assert on.

function on_ready(self)
  self.arrivals = 0
  self.last_jitter = -1
end

function on_arrived(self, from, name, payload)
  self.arrivals = self.arrivals + 1
  self.last_jitter = payload.jitter
end

function on_post_tick(self)
  self.enemies = #scene.tagged("enemy")
end
