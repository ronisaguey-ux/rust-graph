Imports System
Imports System.Text

Module Program

    Sub Main()
        Dim total As Integer = 0
        Dim greeting As String = "hello"
        For index As Integer = 1 To 10
            total = total + index
        Next
        If total > 10 Then
            Console.WriteLine(greeting)
        ElseIf total = 0 Then
            Console.WriteLine("zero")
        Else
            Console.WriteLine("small")
        End If
        While total > 0
            total = total - 1
        End While
    End Sub

End Module
