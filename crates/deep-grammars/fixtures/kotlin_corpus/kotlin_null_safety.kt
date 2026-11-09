package com.example.safe

data class Address(val city: String?)
data class Person(val name: String, val address: Address?)

fun cityOf(person: Person?): String {
    return person?.address?.city ?: "unknown"
}

fun length(text: String?): Int = text?.trim()?.length ?: 0

fun requireCity(person: Person): String {
    val address = person.address ?: return "no address"
    val city = address.city ?: throw IllegalStateException("missing city")
    return city
}

fun upper(text: String?): String? = text?.let { it.trim().uppercase() }

fun force(value: String?): String = value!!

fun chain(values: Map<String, List<Int>>, key: String): Int =
    values[key]?.firstOrNull()?.takeIf { it > 0 } ?: 0

fun cast(value: Any?): String? {
    if (value is String && value.isNotEmpty()) {
        return value
    }
    return (value as? String)?.trim()
}

fun firstName(person: Person): String = person.address?.city?.split(" ")?.first() ?: person.name
