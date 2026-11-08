package com.example.parse

sealed class Token {
    data class Word(val text: String) : Token()
    data class Number(val value: Int) : Token()
    object End : Token()
}

class Parser(private val source: String) {
    private var position = 0

    fun parse(): List<Token> {
        val tokens = mutableListOf<Token>()
        while (position < source.length) {
            val char = source[position]
            when {
                char.isWhitespace() -> position++
                char.isDigit() -> tokens.add(readNumber())
                else -> tokens.add(readWord())
            }
        }
        tokens.add(Token.End)
        return tokens
    }

    private fun readNumber(): Token.Number {
        val start = position
        while (position < source.length && source[position].isDigit()) {
            position++
        }
        return Token.Number(source.substring(start, position).toInt())
    }

    private fun readWord(): Token.Word {
        val start = position
        while (position < source.length && !source[position].isWhitespace()) {
            position++
        }
        return Token.Word(source.substring(start, position))
    }
}
