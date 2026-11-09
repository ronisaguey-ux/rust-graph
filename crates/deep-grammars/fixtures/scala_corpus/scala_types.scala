package com.example.types

trait Codec[A] {
  def encode(value: A): String
  def decode(text: String): Either[String, A]
}

object Codec {
  def apply[A](implicit codec: Codec[A]): Codec[A] = codec

  implicit val intCodec: Codec[Int] = new Codec[Int] {
    def encode(value: Int): String = value.toString

    def decode(text: String): Either[String, Int] =
      try Right(text.toInt) catch { case _: NumberFormatException => Left(text) }
  }
}

class Box[+A](val value: A) {
  def map[B](f: A => B): Box[B] = new Box[B](f(value))
  def flatMap[B](f: A => Box[B]): Box[B] = f(value)
}

object Boxes {
  def combine[A, B](left: List[A], right: List[B]): List[(A, B)] =
    left.flatMap(a => right.map(b => (a, b)))

  def maxOf[A <: Comparable[A]](values: List[A]): A =
    values.reduceLeft((left, right) => if (left.compareTo(right) >= 0) left else right)
}
