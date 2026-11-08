# A service object: state, a registry and method calls.
require "logger"

class Service
  LIMIT = 10

  def initialize(logger)
    @logger = logger
    @handlers = {}
    @count = 0
  end

  def register(name, handler)
    @handlers[name] = handler
    @count += 1
    self
  end

  def call(event)
    handler = @handlers[event.name]
    if handler.nil?
      return nil
    end
    handler.call(event)
  end

  def size
    @count
  end
end

logger = Logger.new("app.log")
service = Service.new(logger)
service.register("tick", logger)
puts(service.size)
