# Shapes: a module, a class hierarchy and their methods.
module Geometry
  PI = 3.14159

  def self.area(radius)
    PI * radius ** 2
  end
end

class Shape
  include Geometry

  def initialize(name)
    @name = name
    @sides = 0
  end

  def name
    @name
  end

  def describe
    "#{@name} with #{@sides} sides"
  end
end

class Circle < Shape
  def initialize(radius)
    super(radius)
    @radius = radius
  end

  def area
    Geometry::PI * @radius ** 2
  end
end
