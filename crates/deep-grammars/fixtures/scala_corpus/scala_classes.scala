package com.example.domain

import scala.collection.mutable

sealed abstract class Vehicle(val wheels: Int) {
  def describe: String = s"$wheels wheels"
}

class Car extends Vehicle(4) {
  override def describe: String = "car with " + super.describe
}

case class Truck(axles: Int) extends Vehicle(axles * 2)

object Garage {
  private val stored = mutable.ListBuffer.empty[Vehicle]

  def park(vehicle: Vehicle): Unit = {
    stored += vehicle
  }

  def byWheels(count: Int): List[Vehicle] =
    stored.filter(vehicle => vehicle.wheels == count).toList

  def summary: String = stored.map(vehicle => vehicle.describe).mkString("; ")

  def heaviest: Option[Vehicle] =
    if (stored.isEmpty) None else Some(stored.maxBy(vehicle => vehicle.wheels))
}
