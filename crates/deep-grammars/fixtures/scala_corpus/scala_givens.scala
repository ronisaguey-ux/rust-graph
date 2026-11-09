package com.example.typeclass

trait Show[A] {
  def show(value: A): String
}

object Show {
  def apply[A](implicit instance: Show[A]): Show[A] = instance

  implicit val intShow: Show[Int] = new Show[Int] {
    def show(value: Int): String = value.toString
  }

  implicit val stringShow: Show[String] = new Show[String] {
    def show(value: String): String = "\"" + value + "\""
  }

  implicit def listShow[A](implicit inner: Show[A]): Show[List[A]] = new Show[List[A]] {
    def show(value: List[A]): String = value.map(inner.show).mkString("[", ", ", "]")
  }
}

object ShowSyntax {
  def render[A](value: A)(implicit show: Show[A]): String = show.show(value)
}

object Modern {
  given intShow: Show[Int] with {
    def show(value: Int): String = value.toString
  }

  given Show[String] = (value: String) => "\"" + value + "\""

  def renderAll[A](values: List[A])(using show: Show[A]): String =
    values.map(show.show).mkString(", ")
}
