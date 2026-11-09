package com.example.pattern

sealed trait Shape
case class Circle(radius: Double) extends Shape
case class Rect(width: Double, height: Double) extends Shape
case class Polygon(sides: List[Double]) extends Shape

object Area {
  def of(shape: Shape): Double = shape match {
    case Circle(r) => math.Pi * r * r
    case Rect(w, h) => w * h
    case Polygon(s :: rest) => s * rest.length
    case Polygon(Nil) => 0.0
  }

  def label(value: Any): String = value match {
    case n: Int if n > 0 => "positive int"
    case n: Int => "int"
    case text: String => text
    case (a, b) => s"pair $a $b"
    case Nil => "empty"
    case _ => "other"
  }

  def unwrap(option: Option[Int]): Int = option match {
    case Some(value) => value
    case None => -1
  }
}
