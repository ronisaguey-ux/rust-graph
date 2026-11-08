C     Broken: the IF is never closed and the call loses its paren.
      PROGRAM BROKEN
      IMPLICIT NONE
      INTEGER N
      N = 3
      IF (N .GT. 0) THEN
         PRINT *, 'POSITIVE'
      CALL REPORT(N
      END
