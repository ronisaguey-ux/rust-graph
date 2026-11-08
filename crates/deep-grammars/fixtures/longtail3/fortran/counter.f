C     Loops, strings and a character declaration.
      PROGRAM COUNTER
      IMPLICIT NONE
      INTEGER I, J, SUM
      CHARACTER NAME
      NAME = 'COUNTER'
      SUM = 0
      DO I = 1, 5
         DO J = 1, I
            SUM = SUM + J
         END DO
      END DO
      WRITE (*,*) NAME, SUM
      END PROGRAM COUNTER
