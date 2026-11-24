- **Revit 2008 families draw their forms (measured on two private 2008
  families that drew nothing).** Revit 2008 writes its data objects with no
  ElementId: an object is Adler-32, size, payload and size. Read with a
  `u32` id in front, each object took the one before's trailing size for its
  id and overlapped it, so every second object went unread (929 of one
  family's objects were gaps, 1 now), and the rest were keyed by that size
  instead of their element's id: no sweep found its sketches. A 2008 object
  now takes its root's `m_id` as its id, and a 2008 sweep's profile fields
  (profile family, its plane, offsets, flip), kept on the sweep itself rather
  than in `m_profileData`, are read. The two families go from 0 of 12 and 0
  of 6 forms drawn to 21 of 24 (3 voids, all cutting) and 12 of 12. Files from
  2010 on are framed as before (every object's id equal to its `m_id` on 633
  published families and 65 manufacturer families).
- **Spline lines in family sketches are drawn as splines.** A sketch line
  drawn as a spline keeps the spline in its control data (a Hermite spline or
  a NURBS curve) and a straight line between its ends as its curve; the
  straight line was drawn, so a chair's bent tubes came out straight. The
  spline is now evaluated and thinned to the points its shape needs. On 65
  manufacturer families 12 gain 13 solved extrusions that were boxes, one
  nested box becomes its solid, and every overall size stays the same (60 of
  61 families with a CAD view still within 1% of it).
- **A sweep whose path crosses its profile's plane more than once is swept
  from the segment beside the profile.** The first crossing was taken, so a
  U-shaped seat cushion was pushed along the opposite side of the U and
  stood outside the seat it belongs to.
