require "json"

class Good
  def value : Int32
    1
  end
end

class Broken
  def render(name : String) : String
    if name.empty?
      "empty"
  end
end

puts "unterminated
