Imports System

Public Class Report

    Private Title As String
    Private Count As Integer
    Private Lines As String

    Public Sub New(title As String)
        Me.Title = title
        Me.Count = 0
    End Sub

    Public Function Describe() As String
        Return Me.Title + " (" + Me.Count.ToString() + ")"
    End Function

    Public Sub Run()
        Try
            For Each line As String In Me.Lines
                Me.Count = Me.Count + 1
            Next
        Catch error As Exception
            Console.WriteLine(error.Message)
        Finally
            Console.WriteLine("done")
        End Try
    End Sub

End Class
