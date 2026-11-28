<!-- markdownlint-disable-file MD041 -->
- **Baked objects' corners are written once a place.** A tile's copied trees and ferns had a vertex for every corner of every
  triangle; corners at the same place with the same texture coordinates are now one vertex, each face keeping its own flat
  normal. Writing and compiling the forest showcase's tile models took 63 s of thread time instead of 92 to 105 s. The models
  draw the same and the hak is the same size.
