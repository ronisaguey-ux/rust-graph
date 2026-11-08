require "http/server"

module Server
  VERSION = "1.2.0"

  class Handler
    include HTTP::Handler

    @count : Int32 = 0
    @prefix : String

    def initialize(@prefix : String)
      @count = 0
    end

    def render(name : String) : String
      @count += 1
      "#{@prefix}/#{name}"
    end

    def hits : Int32
      @count
    end
  end
end

server = HTTP::Server.new(Server::Handler.new("api"))
port = 8080

if port > 1024
  puts "listening"
else
  puts "privileged port"
end

server.listen(port)
