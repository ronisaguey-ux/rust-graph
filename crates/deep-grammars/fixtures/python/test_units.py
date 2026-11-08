import unittest
from decimal import Decimal

from shop.cart import Cart, EmptyCart


class CartTest(unittest.TestCase):
    def setUp(self):
        self.cart = Cart(tax_rate=Decimal("0.2"))

    def test_empty_cart_total_is_zero(self):
        self.assertEqual(self.cart.total(), Decimal("0"))

    def test_add_one_line(self):
        self.cart.add("apple", Decimal("1.50"), 2)
        self.assertEqual(self.cart.total(), Decimal("3.60"))

    def test_removing_everything(self):
        self.cart.add("pear", Decimal("2"), 1)
        self.cart.remove("pear")
        with self.assertRaises(EmptyCart):
            self.cart.checkout()

    def test_discounts_stack(self):
        for sku, price in [("a", 10), ("b", 20)]:
            self.cart.add(sku, Decimal(price), 1)
        discounted = self.cart.apply_discount(Decimal("0.1"))
        self.assertLess(discounted.total(), self.cart.total())


if __name__ == "__main__":
    unittest.main()
