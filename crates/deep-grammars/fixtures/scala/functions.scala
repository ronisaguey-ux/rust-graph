object Functions {
  def greet(name: String, greeting: String = "Hello"): String = {
    s"$greeting, $name!"
  }

  def add(a: Int, b: Int): Int = a + b

  def applyTwice[A](value: A, f: A => A): A = f(f(value))

  def sum(values: Int*): Int = values.sum

  val numbers = List(1, 2, 3)
  val doubled = numbers.map(x => x * 2)
  val kept = numbers.filter(value => value > 1)
  val total = numbers.foldLeft(0)((acc, n) => acc + n)
}
