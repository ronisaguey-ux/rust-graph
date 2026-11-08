Namespace Services

    Public Class Router

        Private Visits As Integer

        Public Sub Dispatch(code As Integer)
            Select Case code
                Case 1
                    Console.WriteLine("one")
                Case 2, 3
                    Console.WriteLine("few")
                Case Else
                    Console.WriteLine("many")
            End Select
            Do While code > 0
                code = code - 1
            Loop
        End Sub

    End Class

End Namespace
