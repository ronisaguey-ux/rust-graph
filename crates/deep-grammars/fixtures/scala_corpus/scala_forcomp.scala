package com.example.loops

case class Item(name: String, price: Double, tags: List[String])

object Loops {
  val items = List(Item("pen", 1.5, List("office")), Item("book", 12.0, List("office", "gift")))

  def cheap(limit: Double): List[String] =
    for (item <- items if item.price < limit) yield item.name

  def pairs: List[(String, String)] =
    for (a <- items; b <- items if a != b) yield (a.name, b.name)

  def total(prices: List[Double]): Double = {
    var sum = 0.0
    for (price <- prices) {
      sum += price
    }
    sum
  }

  def labelled: Map[String, List[String]] =
    items.flatMap(item => item.tags.map(tag => (tag, item.name))).groupBy(pair => pair._1).map {
      case (tag, pairs) => (tag, pairs.map(pair => pair._2))
    }

  def squares(limit: Int): List[Int] =
    for (n <- List(1, 2, 3, 4, 5) if n <= limit) yield n * n
}
