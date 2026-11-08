*     A module of numeric helpers, fixed form.
      MODULE GEOMETRY
      IMPLICIT NONE
      REAL, PARAMETER :: PI = 3.14159
      CONTAINS
      FUNCTION AREA
     1 (RADIUS)
      IMPLICIT NONE
      REAL AREA, RADIUS
      AREA = PI * RADIUS ** 2
      RETURN
      END
      END MODULE

      PROGRAM DRIVER
      USE GEOMETRY, ONLY: AREA
      IMPLICIT NONE
      REAL R, RESULT
      R = 2.0
      RESULT = AREA(R)
      PRINT *, RESULT
      END PROGRAM
