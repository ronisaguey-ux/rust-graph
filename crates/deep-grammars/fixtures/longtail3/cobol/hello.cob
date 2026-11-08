       IDENTIFICATION DIVISION.
       PROGRAM-ID. HELLO.
       DATA DIVISION.
       WORKING-STORAGE SECTION.
       01 WS-NAME PIC X(10) VALUE 'WORLD'.
       01 WS-COUNT PIC 9(3) VALUE ZERO.
       PROCEDURE DIVISION.
       MAIN-PARA.
           DISPLAY 'HELLO ' WS-NAME.
           MOVE 5 TO WS-COUNT.
           COMPUTE WS-COUNT = WS-COUNT + 1.
           IF WS-COUNT > 0 THEN
               DISPLAY 'POSITIVE'
           ELSE
               DISPLAY 'ZERO'
           END-IF.
           STOP RUN.
