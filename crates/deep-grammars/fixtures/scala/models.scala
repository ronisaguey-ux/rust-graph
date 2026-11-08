package com.example.models

import scala.collection.mutable
import java.time.Instant

trait Shape {
  def name: String
  def area: Double
}

case class Circle(name: String, radius: Double) extends Shape {
  def area: Double = 3.14159 * radius * radius
}

class Rectangle(val name: String, val width: Double, val height: Double) extends Shape {
  override def area: Double = width * height
}

object Shapes {
  def largest(shapes: List[Shape]): Shape = {
    shapes.maxBy(shape => shape.area)
  }
}
