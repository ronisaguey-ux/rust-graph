C     A fixed-form program: columns 1-6 are the prefix.
      PROGRAM MAIN
      IMPLICIT NONE
      INTEGER N, I
      REAL TOTAL
      TOTAL = 0.0
      N = 10
      DO I = 1, N
         TOTAL = TOTAL + I
      END DO
      IF (TOTAL .GT. 50.0) THEN
         PRINT *, 'BIG', TOTAL
      ELSE
         PRINT *, 'SMALL'
      END IF
      CALL REPORT(TOTAL)
      STOP
      END

      SUBROUTINE REPORT
     1 (VALUE)
      IMPLICIT NONE
      REAL VALUE
      PRINT *, VALUE
      RETURN
      END
